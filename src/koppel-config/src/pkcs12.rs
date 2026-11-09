//! Reads the PKCS#12 files .NET exports with `X509Certificate2.Export(X509ContentType.Pfx)` and no
//! password, which is how Skakel 0.1 and Koppel 0.1.0 kept the server certificate.
//!
//! .NET writes them the same on Windows and Linux: the key in a shrouded key bag and the
//! certificate in an encrypted bag, both with pbeWithSHAAnd3-KeyTripleDES-CBC and 2000 iterations,
//! and an HMAC-SHA1 over the lot. "No password" there is the empty BMPString without its two zero
//! terminator bytes, which the Rust PKCS#12 crates cannot express (`p12-keystore` and `p12` both fail
//! the MAC on these files), so this reads the format itself: a DER walk, the RFC 7292 appendix B key
//! derivation over SHA-1, and 3DES-CBC. The empty password with the terminator is tried as well.
//!
//! [`write`] makes a file of the same shape, so a certificate Koppel 0.2 makes is kept the way
//! 0.1.x kept its own.

use std::fmt;

use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{digest, hmac};

const OID_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
const OID_ENCRYPTED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x06];
const OID_KEY_BAG: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x0A, 0x01, 0x01,
];
const OID_SHROUDED_KEY_BAG: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x0A, 0x01, 0x02,
];
const OID_CERT_BAG: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x0A, 0x01, 0x03,
];
const OID_X509_CERTIFICATE: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x16, 0x01];
const OID_LOCAL_KEY_ID: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x15];
const OID_PBE_SHA1_3DES: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x01, 0x03];
const OID_SHA1: &[u8] = &[0x2B, 0x0E, 0x03, 0x02, 0x1A];

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const CONTEXT_0: u8 = 0xA0;
const CONTEXT_0_PRIMITIVE: u8 = 0x80;

/// More than any real file uses, and few enough that a hostile one cannot stall a start.
const MAX_ITERATIONS: u64 = 1_000_000;
/// What .NET uses for the bags and the MAC when it exports a certificate.
const WRITE_ITERATIONS: u64 = 2000;

/// Why a PKCS#12 file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkcs12Error(pub String);

impl fmt::Display for Pkcs12Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Pkcs12Error {}

fn error<T>(what: impl Into<String>) -> Result<T, Pkcs12Error> {
    Err(Pkcs12Error(what.into()))
}

/// A private key or certificate from a bag, with the `localKeyId` that pairs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bag {
    pub der: Vec<u8>,
    pub local_key_id: Option<Vec<u8>>,
}

/// What a PKCS#12 file holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pkcs12 {
    /// Private keys, PKCS#8 DER.
    pub keys: Vec<Bag>,
    /// Certificates, DER.
    pub certificates: Vec<Bag>,
}

/// A DER reader over a slice.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn peek_tag(&self) -> Option<u8> {
        self.0.first().copied()
    }

    fn any(&mut self) -> Result<(u8, &'a [u8]), Pkcs12Error> {
        let data = self.0;
        let (&tag, rest) = data
            .split_first()
            .ok_or(Pkcs12Error("the DER ends early".into()))?;
        let (&first, rest) = rest
            .split_first()
            .ok_or(Pkcs12Error("the DER ends early".into()))?;
        let (length, rest) = if first < 0x80 {
            (usize::from(first), rest)
        } else {
            let count = usize::from(first & 0x7F);
            if count == 0 || count > 4 || rest.len() < count {
                return error("the DER has a length it cannot have");
            }
            let length = rest[..count]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
            (length, &rest[count..])
        };
        if rest.len() < length {
            return error("the DER ends early");
        }
        self.0 = &rest[length..];
        Ok((tag, &rest[..length]))
    }

    fn expect(&mut self, want: u8, what: &str) -> Result<&'a [u8], Pkcs12Error> {
        let (tag, value) = self.any()?;
        if tag != want {
            return error(format!("expected {what}, found tag {tag:#04x}"));
        }
        Ok(value)
    }

    fn sequence(&mut self, what: &str) -> Result<Der<'a>, Pkcs12Error> {
        self.expect(SEQUENCE, what).map(Der)
    }

    fn integer(&mut self) -> Result<u64, Pkcs12Error> {
        let value = self.expect(INTEGER, "an integer")?;
        if value.is_empty() || value.len() > 8 || value[0] & 0x80 != 0 {
            return error("an integer is out of range");
        }
        Ok(value.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
    }
}

/// Reads `pfx`, trying "no password" and then the empty password.
pub fn parse(pfx: &[u8]) -> Result<Pkcs12, Pkcs12Error> {
    let mut outer = Der(pfx);
    let mut pfx_seq = outer.sequence("the PFX")?;
    if pfx_seq.integer()? != 3 {
        return error("it is not PKCS#12 version 3");
    }
    let mut auth_safe = pfx_seq.sequence("the authenticated safe")?;
    if auth_safe.expect(OID, "a content type")? != OID_DATA {
        return error(
            "the authenticated safe is signed, not password protected; .NET never writes that",
        );
    }
    let content =
        Der(auth_safe.expect(CONTEXT_0, "the content")?).expect(OCTET_STRING, "the content")?;
    let passwords: [&[u8]; 2] = [&[], &[0, 0]];
    let password = if pfx_seq.is_empty() {
        None
    } else {
        let mut mac_data = pfx_seq.sequence("the MAC")?;
        let mut digest_info = mac_data.sequence("the MAC digest")?;
        let mut algorithm = digest_info.sequence("the MAC algorithm")?;
        if algorithm.expect(OID, "the MAC algorithm")? != OID_SHA1 {
            return error("the MAC is not HMAC-SHA1; .NET 0.1.x always wrote HMAC-SHA1");
        }
        let expected = digest_info.expect(OCTET_STRING, "the MAC")?;
        let salt = mac_data.expect(OCTET_STRING, "the MAC salt")?;
        let iterations = if mac_data.is_empty() {
            1
        } else {
            mac_data.integer()?
        };
        let found = passwords.iter().find(|password| {
            kdf(password, salt, 3, iterations, 20).is_ok_and(|key| {
                hmac::verify(
                    &hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &key),
                    content,
                    expected,
                )
                .is_ok()
            })
        });
        Some(*found.ok_or(Pkcs12Error(
            "its MAC does not match: it has a password".into(),
        ))?)
    };

    let mut out = Pkcs12::default();
    let mut safes = Der(content).sequence("the safe contents")?;
    while !safes.is_empty() {
        let mut info = safes.sequence("a content")?;
        let kind = info.expect(OID, "a content type")?;
        let body = info.expect(CONTEXT_0, "a content")?;
        let contents = if kind == OID_DATA {
            Der(body).expect(OCTET_STRING, "the data")?.to_vec()
        } else if kind == OID_ENCRYPTED_DATA {
            let mut encrypted = Der(body).sequence("the encrypted data")?;
            encrypted.integer()?;
            let mut content_info = encrypted.sequence("the encrypted content")?;
            content_info.expect(OID, "the content type")?;
            let algorithm = content_info.any()?;
            let ciphertext = content_info.expect(CONTEXT_0_PRIMITIVE, "the ciphertext")?;
            decrypt_with(password, algorithm, ciphertext)?
        } else {
            return error("a content is neither data nor encrypted data");
        };
        read_bags(&contents, password, &mut out)?;
    }
    if out.keys.is_empty() || out.certificates.is_empty() {
        return error("it does not hold both a certificate and its private key");
    }
    Ok(out)
}

fn read_bags(
    contents: &[u8],
    password: Option<&[u8]>,
    out: &mut Pkcs12,
) -> Result<(), Pkcs12Error> {
    let mut bags = Der(contents).sequence("the bags")?;
    while !bags.is_empty() {
        let mut bag = bags.sequence("a bag")?;
        let kind = bag.expect(OID, "a bag type")?;
        let value = bag.expect(CONTEXT_0, "a bag value")?;
        let local_key_id = if bag.peek_tag() == Some(SET) {
            local_key_id(bag.expect(SET, "attributes")?)?
        } else {
            None
        };
        if kind == OID_KEY_BAG {
            out.keys.push(Bag {
                der: Der(value)
                    .expect(SEQUENCE, "a key")
                    .map(|_| value.to_vec())?,
                local_key_id,
            });
        } else if kind == OID_SHROUDED_KEY_BAG {
            let mut encrypted = Der(value).sequence("an encrypted key")?;
            let algorithm = encrypted.any()?;
            let ciphertext = encrypted.expect(OCTET_STRING, "an encrypted key")?;
            out.keys.push(Bag {
                der: decrypt_with(password, algorithm, ciphertext)?,
                local_key_id,
            });
        } else if kind == OID_CERT_BAG {
            let mut cert_bag = Der(value).sequence("a certificate bag")?;
            if cert_bag.expect(OID, "a certificate type")? == OID_X509_CERTIFICATE {
                let der = Der(cert_bag.expect(CONTEXT_0, "a certificate")?)
                    .expect(OCTET_STRING, "a certificate")?;
                out.certificates.push(Bag {
                    der: der.to_vec(),
                    local_key_id,
                });
            }
        }
    }
    Ok(())
}

fn local_key_id(attributes: &[u8]) -> Result<Option<Vec<u8>>, Pkcs12Error> {
    let mut attributes = Der(attributes);
    while !attributes.is_empty() {
        let mut attribute = attributes.sequence("an attribute")?;
        if attribute.expect(OID, "an attribute type")? == OID_LOCAL_KEY_ID {
            let mut values = Der(attribute.expect(SET, "attribute values")?);
            return Ok(Some(
                values.expect(OCTET_STRING, "a local key id")?.to_vec(),
            ));
        }
    }
    Ok(None)
}

/// Decrypts a pbeWithSHAAnd3-KeyTripleDES-CBC ciphertext. Without a MAC to tell which, both
/// passwords are tried.
fn decrypt_with(
    password: Option<&[u8]>,
    algorithm: (u8, &[u8]),
    ciphertext: &[u8],
) -> Result<Vec<u8>, Pkcs12Error> {
    let (tag, algorithm) = algorithm;
    if tag != SEQUENCE {
        return error("an encryption algorithm is not a sequence");
    }
    let mut algorithm = Der(algorithm);
    let oid = algorithm.expect(OID, "an encryption algorithm")?;
    if oid != OID_PBE_SHA1_3DES {
        return error(
            "a bag is encrypted with something other than pbeWithSHAAnd3-KeyTripleDES-CBC, which .NET 0.1.x always used",
        );
    }
    let mut parameters = algorithm.sequence("the PBE parameters")?;
    let salt = parameters.expect(OCTET_STRING, "the PBE salt")?;
    let iterations = parameters.integer()?;
    let tries: Vec<&[u8]> = match password {
        Some(password) => vec![password],
        None => vec![&[], &[0, 0]],
    };
    for password in tries {
        let key = kdf(password, salt, 1, iterations, 24)?;
        let iv = kdf(password, salt, 2, iterations, 8)?;
        let mut buffer = ciphertext.to_vec();
        if let Ok(plain) = cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
            .expect("key and IV lengths")
            .decrypt_padded::<Pkcs7>(&mut buffer)
        {
            return Ok(plain.to_vec());
        }
    }
    error("a bag does not decrypt: it has a password")
}

/// RFC 7292 appendix B.2 over SHA-1: `id` 1 for a key, 2 for an IV, 3 for a MAC key. `password`
/// is the BMPString with its terminator, or empty for no password at all.
fn kdf(
    password: &[u8],
    salt: &[u8],
    id: u8,
    iterations: u64,
    length: usize,
) -> Result<Vec<u8>, Pkcs12Error> {
    const V: usize = 64;
    if iterations == 0 || iterations > MAX_ITERATIONS {
        return error(format!("it asks for {iterations} iterations"));
    }
    let stretch = |input: &[u8]| -> Vec<u8> {
        if input.is_empty() {
            return Vec::new();
        }
        input
            .iter()
            .copied()
            .cycle()
            .take(V * input.len().div_ceil(V))
            .collect()
    };
    let mut i = stretch(salt);
    i.extend(stretch(password));
    let mut out = Vec::with_capacity(length + 20);
    loop {
        let mut context = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
        context.update(&[id; V]);
        context.update(&i);
        let mut a = context.finish();
        for _ in 1..iterations {
            a = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, a.as_ref());
        }
        out.extend_from_slice(a.as_ref());
        if out.len() >= length {
            out.truncate(length);
            return Ok(out);
        }
        let b: Vec<u8> = a.as_ref().iter().copied().cycle().take(V).collect();
        for block in i.chunks_mut(V) {
            let mut carry = 1u16;
            for k in (0..V).rev() {
                let sum = u16::from(block[k]) + u16::from(b[k]) + carry;
                block[k] = sum as u8;
                carry = sum >> 8;
            }
        }
    }
}

/// A DER element: the tag, the length and the content.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = content.len();
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes: Vec<u8> = length
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    tlv(SEQUENCE, &parts.concat())
}

fn der_integer(value: u64) -> Vec<u8> {
    let mut bytes: Vec<u8> = value
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    if bytes.first().is_none_or(|b| b & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    tlv(INTEGER, &bytes)
}

fn random_bytes<const N: usize>(random: &SystemRandom) -> Result<[u8; N], Pkcs12Error> {
    let mut bytes = [0u8; N];
    random
        .fill(&mut bytes)
        .map_err(|_| Pkcs12Error("the system gave no random bytes".into()))?;
    Ok(bytes)
}

/// `plain` encrypted under "no password", with the pbeWithSHAAnd3-KeyTripleDES-CBC algorithm
/// identifier that says how.
fn encrypt(plain: &[u8], random: &SystemRandom) -> Result<(Vec<u8>, Vec<u8>), Pkcs12Error> {
    let salt: [u8; 8] = random_bytes(random)?;
    let key = kdf(&[], &salt, 1, WRITE_ITERATIONS, 24)?;
    let iv = kdf(&[], &salt, 2, WRITE_ITERATIONS, 8)?;
    let mut buffer = plain.to_vec();
    buffer.resize(plain.len() + 8, 0);
    let ciphertext = cbc::Encryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
        .expect("key and IV lengths")
        .encrypt_padded::<Pkcs7>(&mut buffer, plain.len())
        .map_err(|_| Pkcs12Error("the padding did not fit".into()))?
        .to_vec();
    let algorithm = sequence(&[
        &tlv(OID, OID_PBE_SHA1_3DES),
        &sequence(&[&tlv(OCTET_STRING, &salt), &der_integer(WRITE_ITERATIONS)]),
    ]);
    Ok((algorithm, ciphertext))
}

/// Writes a certificate (DER) and its private key (PKCS#8 DER) as .NET exports them without a
/// password: the certificate in an encrypted bag, the key in a shrouded key bag, both 3DES with
/// 2000 iterations, paired by a `localKeyId`, and an HMAC-SHA1 over the lot.
pub fn write(certificate: &[u8], private_key: &[u8]) -> Result<Vec<u8>, Pkcs12Error> {
    let random = SystemRandom::new();
    let local_key_id = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, certificate);
    let attributes = tlv(
        SET,
        &sequence(&[
            &tlv(OID, OID_LOCAL_KEY_ID),
            &tlv(SET, &tlv(OCTET_STRING, local_key_id.as_ref())),
        ]),
    );

    let cert_bag = sequence(&[
        &tlv(OID, OID_CERT_BAG),
        &tlv(
            CONTEXT_0,
            &sequence(&[
                &tlv(OID, OID_X509_CERTIFICATE),
                &tlv(CONTEXT_0, &tlv(OCTET_STRING, certificate)),
            ]),
        ),
        &attributes,
    ]);
    let (algorithm, ciphertext) = encrypt(&sequence(&[&cert_bag]), &random)?;
    let certificates = sequence(&[
        &tlv(OID, OID_ENCRYPTED_DATA),
        &tlv(
            CONTEXT_0,
            &sequence(&[
                &der_integer(0),
                &sequence(&[
                    &tlv(OID, OID_DATA),
                    &algorithm,
                    &tlv(CONTEXT_0_PRIMITIVE, &ciphertext),
                ]),
            ]),
        ),
    ]);

    let (algorithm, ciphertext) = encrypt(private_key, &random)?;
    let key_bag = sequence(&[
        &tlv(OID, OID_SHROUDED_KEY_BAG),
        &tlv(
            CONTEXT_0,
            &sequence(&[&algorithm, &tlv(OCTET_STRING, &ciphertext)]),
        ),
        &attributes,
    ]);
    let keys = sequence(&[
        &tlv(OID, OID_DATA),
        &tlv(CONTEXT_0, &tlv(OCTET_STRING, &sequence(&[&key_bag]))),
    ]);

    let content = sequence(&[&certificates, &keys]);
    let mac_salt: [u8; 20] = random_bytes(&random)?;
    let mac_key = kdf(&[], &mac_salt, 3, WRITE_ITERATIONS, 20)?;
    let mac = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &mac_key),
        &content,
    );
    let null = [0x05, 0x00];
    let mac_data = sequence(&[
        &sequence(&[
            &sequence(&[&tlv(OID, OID_SHA1), &null]),
            &tlv(OCTET_STRING, mac.as_ref()),
        ]),
        &tlv(OCTET_STRING, &mac_salt),
        &der_integer(WRITE_ITERATIONS),
    ]);
    Ok(sequence(&[
        &der_integer(3),
        &sequence(&[
            &tlv(OID, OID_DATA),
            &tlv(CONTEXT_0, &tlv(OCTET_STRING, &content)),
        ]),
        &mac_data,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_write_makes_reads_back() {
        let certificate = [0x30, 0x03, 0x02, 0x01, 0x07].repeat(40);
        let key = vec![0x30, 0x05, 0x02, 0x03, 0x01, 0x02, 0x03];
        let pfx = write(&certificate, &key).unwrap();
        let read = parse(&pfx).unwrap();
        assert_eq!(read.certificates[0].der, certificate);
        assert_eq!(read.keys[0].der, key);
        assert!(read.keys[0].local_key_id.is_some());
        assert_eq!(read.keys[0].local_key_id, read.certificates[0].local_key_id);
    }

    #[test]
    fn the_key_derivation_matches_a_known_answer() {
        // From the PKCS#12 test vectors (password "smeg", salt 0A58CF64530D823F, 1 iteration).
        let password: Vec<u8> = "smeg"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .chain([0, 0])
            .collect();
        let salt = [0x0A, 0x58, 0xCF, 0x64, 0x53, 0x0D, 0x82, 0x3F];
        let key = kdf(&password, &salt, 1, 1, 24).unwrap();
        assert_eq!(
            key,
            [
                0x8A, 0xAA, 0xE6, 0x29, 0x7B, 0x6C, 0xB0, 0x46, 0x42, 0xAB, 0x5B, 0x07, 0x78, 0x51,
                0x28, 0x4E, 0xB7, 0x12, 0x8F, 0x1A, 0x2A, 0x7F, 0xBC, 0xA3
            ]
        );
        let iv = kdf(&password, &salt, 2, 1, 8).unwrap();
        assert_eq!(iv, [0x79, 0x99, 0x3D, 0xFE, 0x04, 0x8D, 0x3B, 0x76]);
    }

    #[test]
    fn rubbish_is_refused() {
        assert!(parse(b"").is_err());
        assert!(parse(&[0x30, 0x03, 0x02, 0x01, 0x03]).is_err());
        assert!(parse(&[0x30, 0x84, 0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    }
}
