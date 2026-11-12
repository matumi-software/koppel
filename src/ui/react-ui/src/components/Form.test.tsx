import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Button } from './Button';
import { Field } from './Field';
import { Form, LabeledValue } from './Form';

const tooShort = (value: string) => (value.length < 4 ? 'Use at least 4 characters.' : null);

describe('Form', () => {
  it('checks the rules of a field when it is sent, shows the message in place of the help, and does not send', async () => {
    const onSubmit = vi.fn((event: { preventDefault: () => void }) => event.preventDefault());
    render(
      <Form label="Test" onSubmit={onSubmit}>
        <Field label="Code" name="code" help="At least 4 characters." validate={tooShort} />
        <Button type="submit">Send</Button>
      </Form>,
    );
    await userEvent.type(screen.getByLabelText('Code'), 'ab');
    expect(screen.getByText('At least 4 characters.')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByText('Use at least 4 characters.')).toBeTruthy();
    expect(screen.queryByText('At least 4 characters.')).toBeNull();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('sends once the rules hold, after a correction', async () => {
    const onSubmit = vi.fn((event: { preventDefault: () => void }) => event.preventDefault());
    render(
      <Form label="Test" onSubmit={onSubmit}>
        <Field label="Code" name="code" validate={tooShort} />
        <Button type="submit">Send</Button>
      </Form>,
    );
    await userEvent.type(screen.getByLabelText('Code'), 'ab');
    await userEvent.click(screen.getByRole('button', { name: 'Send' }));
    await userEvent.type(screen.getByLabelText('Code'), 'cd');

    await userEvent.click(screen.getByRole('button', { name: 'Send' }));

    expect(onSubmit).toHaveBeenCalledOnce();
  });

  it('shows a refusal from the server under the field it names, and clears it when the person edits', async () => {
    render(
      <Form label="Test" validationErrors={{ code: 'That code is not right.' }}>
        <Field label="Code" name="code" help="Six digits." />
      </Form>,
    );
    expect(await screen.findByText('That code is not right.')).toBeTruthy();
    expect(screen.getByLabelText('Code').getAttribute('aria-invalid')).toBe('true');

    await userEvent.type(screen.getByLabelText('Code'), '1');
    await userEvent.tab();

    expect(screen.queryByText('That code is not right.')).toBeNull();
    expect(screen.getByText('Six digits.')).toBeTruthy();
  });

  it('is named for screen readers', () => {
    render(
      <Form label="Change password">
        <Field label="Code" name="code" />
      </Form>,
    );

    expect(screen.getByRole('form', { name: 'Change password' })).toBeTruthy();
  });
});

describe('Field outside a form', () => {
  it('shows the answer of the server as its help when it is marked invalid, and still takes a corrected value', async () => {
    const onChange = vi.fn();
    render(<Field label="Code" state="invalid" help="That code is not right." onChange={onChange} />);

    expect(screen.getByText('That code is not right.')).toBeTruthy();
    expect(screen.getByLabelText('Code').getAttribute('aria-invalid')).toBe('true');
    await userEvent.type(screen.getByLabelText('Code'), '1');
    expect(onChange).toHaveBeenCalledWith('1');
  });
});

describe('LabeledValue', () => {
  it('names its value and shows its help', () => {
    render(
      <LabeledValue label="Client ID" help="Copy it to the client.">
        <code>bench-1</code>
      </LabeledValue>,
    );

    expect(screen.getByRole('group', { name: 'Client ID' })).toBeTruthy();
    expect(screen.getByText('Copy it to the client.')).toBeTruthy();
  });
});
