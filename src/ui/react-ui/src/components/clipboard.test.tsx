import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AnnouncerProvider } from './Announcer';
import { copyFailedMessage, copyText } from './clipboard';
import { CopyValue, SecretValue } from './SecretValue';

const setSecureContext = (value: boolean) => Object.defineProperty(window, 'isSecureContext', { value, configurable: true });
const setClipboard = (value: unknown) => Object.defineProperty(navigator, 'clipboard', { value, configurable: true });
const setExecCommand = (value: unknown) => Object.defineProperty(document, 'execCommand', { value, configurable: true });

describe('copyText', () => {
  beforeEach(() => {
    setSecureContext(true);
    setClipboard({ writeText: vi.fn().mockResolvedValue(undefined) });
    setExecCommand(vi.fn().mockReturnValue(true));
  });
  afterEach(() => {
    setClipboard(undefined);
    setExecCommand(undefined);
  });

  it('uses the clipboard API on a secure context', async () => {
    await copyText('abc');

    expect((navigator.clipboard.writeText as ReturnType<typeof vi.fn>)).toHaveBeenCalledWith('abc');
    expect(document.execCommand).not.toHaveBeenCalled();
  });

  it('selects the text and copies the selection when the page is not a secure context', async () => {
    setSecureContext(false);

    await copyText('abc');

    expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
    expect(document.execCommand).toHaveBeenCalledWith('copy');
  });

  it('falls back when the clipboard API does not exist, as on plain HTTP', async () => {
    setClipboard(undefined);

    await copyText('abc');

    expect(document.execCommand).toHaveBeenCalledWith('copy');
  });

  it('falls back when the clipboard API refuses', async () => {
    setClipboard({ writeText: vi.fn().mockRejectedValue(new Error('denied')) });

    await copyText('abc');

    expect(document.execCommand).toHaveBeenCalledWith('copy');
  });

  it('tells the user to press Ctrl+C when nothing can copy, not a TypeError', async () => {
    setSecureContext(false);
    setExecCommand(vi.fn().mockReturnValue(false));

    await expect(copyText('abc')).rejects.toThrow(copyFailedMessage);
    expect(copyFailedMessage).toContain('Ctrl+C');
  });

  it('turns an execCommand that throws into the same message', async () => {
    setSecureContext(false);
    setExecCommand(
      vi.fn(() => {
        throw new TypeError('nope');
      }),
    );

    await expect(copyText('abc')).rejects.toThrow(copyFailedMessage);
  });

  it('leaves no helper field in the page', async () => {
    setSecureContext(false);

    await copyText('abc');

    expect(document.querySelector('textarea')).toBeNull();
  });
});

describe('CopyValue', () => {
  beforeEach(() => {
    setSecureContext(false);
    setClipboard(undefined);
  });
  afterEach(() => setExecCommand(undefined));

  it('copies over plain HTTP through the fallback', async () => {
    setExecCommand(vi.fn().mockReturnValue(true));
    render(<CopyValue value="client-1" />);

    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));

    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
  });

  it('says to press Ctrl+C when the fallback fails', async () => {
    setExecCommand(vi.fn().mockReturnValue(false));
    render(
      <AnnouncerProvider>
        <CopyValue value="client-1" />
      </AnnouncerProvider>,
    );

    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));

    // Shown under the value, and said once through the app's one live region.
    expect(await screen.findAllByText(/Ctrl\+C/)).toHaveLength(2);
    expect((await screen.findByRole('status')).textContent).toContain('Ctrl+C');
  });
});

describe('SecretValue', () => {
  beforeEach(() => {
    setSecureContext(false);
    setClipboard(undefined);
  });
  afterEach(() => setExecCommand(undefined));

  it('copies over plain HTTP through the fallback', async () => {
    setExecCommand(vi.fn().mockReturnValue(true));
    render(<SecretValue label="Client secret" loadSecret={() => Promise.resolve('s3cret')} />);

    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));

    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
  });

  it('shows the secret and says to press Ctrl+C when nothing can copy', async () => {
    setExecCommand(vi.fn().mockReturnValue(false));
    render(
      <AnnouncerProvider>
        <SecretValue label="Client secret" loadSecret={() => Promise.resolve('s3cret')} />
      </AnnouncerProvider>,
    );

    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));

    expect((await screen.findByRole('status')).textContent).toContain('Ctrl+C');
    expect(screen.getByText('s3cret')).toBeTruthy();
  });

  it('names the value with a hidden label, not an aria-label on <code>', () => {
    const { container } = render(<SecretValue label="Client secret" loadSecret={() => Promise.resolve('s3cret')} />);

    expect(container.querySelector('code')!.hasAttribute('aria-label')).toBe(false);
    expect(container.querySelector('.kp-sr-only')!.textContent).toBe('Client secret, hidden');
  });

  it('names the value with the label it is given', () => {
    const { container } = render(<SecretValue label="Admin token" loadSecret={() => Promise.resolve('s3cret')} />);

    expect(container.querySelector('.kp-sr-only')!.textContent).toBe('Admin token, hidden');
  });

  it('drops the secret when it is hidden, and asks again for the next Reveal', async () => {
    const loadSecret = vi.fn(() => Promise.resolve('s3cret'));
    render(<SecretValue label="Client secret" loadSecret={loadSecret} />);

    await userEvent.click(screen.getByRole('button', { name: 'Reveal' }));
    expect(await screen.findByText('s3cret')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Hide' }));

    expect(screen.queryByText('s3cret')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Reveal' }));
    expect(await screen.findByText('s3cret')).toBeTruthy();
    expect(loadSecret).toHaveBeenCalledTimes(2);
  });

  it('does not keep a secret it copied while masked', async () => {
    setExecCommand(vi.fn().mockReturnValue(true));
    const loadSecret = vi.fn(() => Promise.resolve('s3cret'));
    render(<SecretValue label="Client secret" loadSecret={loadSecret} />);

    await userEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await screen.findByRole('button', { name: 'Copied' });
    await userEvent.click(screen.getByRole('button', { name: 'Reveal' }));

    expect(await screen.findByText('s3cret')).toBeTruthy();
    expect(loadSecret).toHaveBeenCalledTimes(2);
  });
});
