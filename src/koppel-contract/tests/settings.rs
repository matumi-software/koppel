//! Settings, the activity log and what the side panel shows about the machine: `/api/settings`,
//! `/api/activity` and `/api/system/*`.

use koppel_contract::shape::{self, Kind};
use koppel_contract::{Body, Main, activity_with, json};
use reqwest::Method;
use serde_json::Value;

async fn settings(main: &Main) -> Value {
    let value = main.admin.get("/api/settings").await.expect(200).json();
    shape::fields(&value, shape::SETTINGS, "GET /api/settings");
    value
}

#[tokio::test]
async fn a_patch_answers_202_and_the_settings_read_back_changed() {
    let Some(main) = Main::get().await else {
        return;
    };
    let before = settings(&main).await;
    assert_eq!(before["role"], json!("server"));

    let answer = main
        .admin
        .patch("/api/settings", json!({ "shareOnStartup": false }))
        .await
        .expect(202);
    answer.empty();
    let after = settings(&main).await;
    assert_eq!(after["shareOnStartup"], json!(false));
    for field in [
        "role",
        "setupComplete",
        "notifyPairing",
        "notifyLink",
        "notifyAttachFailures",
    ] {
        assert_eq!(after[field], before[field], "a patch leaves {field} alone");
    }

    for (field, value) in [
        ("notifyPairing", false),
        ("notifyLink", false),
        ("notifyAttachFailures", false),
        ("setupComplete", true),
    ] {
        main.admin
            .patch("/api/settings", json!({ field: value }))
            .await
            .expect(202);
        assert_eq!(settings(&main).await[field], json!(value), "{field}");
    }
    for role in ["client", "both", "server"] {
        main.admin
            .patch("/api/settings", json!({ "role": role }))
            .await
            .expect(202);
        assert_eq!(settings(&main).await["role"], json!(role));
    }

    // Nothing to change is still accepted.
    main.admin
        .patch("/api/settings", json!({}))
        .await
        .expect(202);
}

#[tokio::test]
async fn a_bad_patch_is_refused_and_changes_nothing() {
    let Some(main) = Main::get().await else {
        return;
    };
    let before = settings(&main).await;
    for body in [
        json!({ "role": "nope" }),
        json!({ "role": 1 }),
        json!({ "role": "Server1" }),
        json!({ "shareOnStartup": "yes" }),
    ] {
        main.admin
            .patch("/api/settings", body.clone())
            .await
            .expect(400);
    }
    main.admin
        .change(
            Method::PATCH,
            "/api/settings",
            Body::Raw("application/json", "{ not json".to_owned()),
        )
        .await
        .expect(400);
    assert_eq!(settings(&main).await, before);
}

#[tokio::test]
async fn a_change_is_one_activity_entry_and_no_change_is_none() {
    let Some(main) = Main::get().await else {
        return;
    };
    main.admin
        .patch("/api/settings", json!({ "shareOnStartup": false }))
        .await
        .expect(202);
    let entries = activity_with(&main.admin, "reconfigured").await;
    let newest = entries.first().expect("a reconfigured entry");
    shape::fields(newest, shape::ACTIVITY, "reconfigured");
    shape::fields(&newest["entity"], shape::ENTITY, "reconfigured.entity");
    assert_eq!(newest["entity"]["type"], json!("machine"));
    assert_eq!(newest["outcome"], json!("succeeded"));

    let count = entries.len();
    main.admin
        .patch("/api/settings", json!({ "shareOnStartup": false }))
        .await
        .expect(202);
    assert_eq!(
        activity_with(&main.admin, "reconfigured").await.len(),
        count,
        "a patch that changes nothing records nothing"
    );
}

/// A time as something that sorts: its digits, the fraction padded. Every time Koppel writes is UTC.
fn sortable(time: &str) -> String {
    assert!(shape::is_time(time), "{time}");
    assert!(
        time.ends_with("+00:00") || time.ends_with('Z') || time.ends_with('z'),
        "UTC: {time}"
    );
    let digits: String = time[19..]
        .strip_prefix('.')
        .map(|f| f.chars().take_while(char::is_ascii_digit).collect())
        .unwrap_or_default();
    format!("{}.{digits:0<9}", &time[..19])
}

#[tokio::test]
async fn activity_is_newest_first_and_takes_a_max_and_an_entity() {
    let Some(main) = Main::get().await else {
        return;
    };
    // At least two entries.
    for share in [false, true] {
        main.admin
            .patch("/api/settings", json!({ "shareOnStartup": share }))
            .await
            .expect(202);
    }

    let all = main.admin.get("/api/activity").await.expect(200).json();
    shape::each(&all, "GET /api/activity", |entry| {
        shape::fields(entry, shape::ACTIVITY, "an activity entry");
        shape::fields(&entry["entity"], shape::ENTITY, "an activity entity");
        assert!(
            ["succeeded", "failed"].contains(&entry["outcome"].as_str().unwrap_or_default()),
            "{entry}"
        );
    });
    let entries = all.as_array().expect("an array");
    assert!(entries.len() >= 2 && entries.len() <= 100, "100 by default");
    let times: Vec<String> = entries
        .iter()
        .map(|e| sortable(e["timestampUtc"].as_str().unwrap_or_default()))
        .collect();
    assert!(
        times.windows(2).all(|pair| pair[0] >= pair[1]),
        "newest first: {times:?}"
    );

    let one = main
        .admin
        .get("/api/activity?max=1")
        .await
        .expect(200)
        .json();
    assert_eq!(one.as_array().map(Vec::len), Some(1));
    assert_eq!(one[0], entries[0]);
    let clamped = main
        .admin
        .get("/api/activity?max=0")
        .await
        .expect(200)
        .json();
    assert_eq!(
        clamped.as_array().map(Vec::len),
        Some(1),
        "max is at least 1"
    );
    let most = main
        .admin
        .get("/api/activity?max=100000")
        .await
        .expect(200)
        .json();
    assert!(most.as_array().map(Vec::len).unwrap_or_default() <= 500);
    main.admin.get("/api/activity?max=many").await.expect(400);

    let id = entries[0]["entity"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let only = main
        .admin
        .get(&format!("/api/activity?entity={id}&max=500"))
        .await
        .expect(200)
        .json();
    let only = only.as_array().expect("an array");
    assert!(!only.is_empty());
    assert!(only.iter().all(|e| e["entity"]["id"] == json!(id)));
    let none = main
        .admin
        .get("/api/activity?entity=no-such-thing")
        .await
        .expect(200)
        .json();
    assert_eq!(none, json!([]));
}

#[tokio::test]
async fn system_info_describes_this_machine_and_its_certificate() {
    let Some(main) = Main::get().await else {
        return;
    };
    let info = main.admin.get("/api/system/info").await.expect(200).json();
    shape::fields(
        &info,
        &[
            ("name", Kind::Str),
            ("version", Kind::Str),
            ("startedAt", Kind::Time),
            ("role", Kind::Str),
            ("uiPort", Kind::Int),
            ("httpsPort", Kind::Int),
            ("fingerprint", Kind::Str),
            ("os", Kind::Str),
            ("tray", Kind::Bool),
        ],
        "GET /api/system/info",
    );
    assert_eq!(info["role"], json!("server"));
    assert_eq!(
        info["fingerprint"],
        json!(main.target.current_certificate().await),
        "the SHA-256 of the certificate the port presents, as upper case hex pairs"
    );

    main.admin
        .patch("/api/settings", json!({ "role": "client" }))
        .await
        .expect(202);
    let as_client = main.admin.get("/api/system/info").await.expect(200).json();
    assert_eq!(as_client["role"], json!("client"));
    assert_eq!(as_client["startedAt"], info["startedAt"]);
}

#[tokio::test]
async fn system_checks_say_whether_this_machine_can_share_and_attach() {
    let Some(main) = Main::get().await else {
        return;
    };
    let checks = main
        .admin
        .get("/api/system/checks")
        .await
        .expect(200)
        .json();
    let mut ids = Vec::new();
    shape::each(&checks, "GET /api/system/checks", |check| {
        shape::fields(
            check,
            &[
                ("id", Kind::Str),
                ("ok", Kind::Bool),
                ("title", Kind::Str),
                ("detail", Kind::Str),
            ],
            "a system check",
        );
        ids.push(check["id"].as_str().unwrap_or_default().to_owned());
    });
    assert!(ids.iter().any(|id| id == "share"), "{ids:?}");
    assert!(ids.iter().any(|id| id == "attach"), "{ids:?}");
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
}
