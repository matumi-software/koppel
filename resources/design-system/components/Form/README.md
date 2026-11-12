# Form, FieldGroup and LabeledValue

- `Form` (`kp-form`) is a react-aria `Form`: fields inside it check their rules when the form is sent, and show a message in place of their help. A rule is a `validate` function or `isRequired` on the field. The server's refusals go to the form's `validationErrors`, one message per field `name`, and clear when the person edits that field. `variant` is a plain column, `card` (a `Card`) or `panel` (the body of a `Panel`); `data-width="narrow"` keeps the form at 420px.
- The send button stays enabled: a button that is off until the form is valid hides the reason. It is off only while the link is not live, and shows its sending state while the request is in flight.
- `FieldGroup` (`kp-fieldgroup`) holds fields that belong together, such as the new password and its repeat, under one name for screen readers, 16px apart.
- `LabeledValue` (`kp-field` with `kp-field__label` and `kp-field__help`) is a value that is not a text input, such as an ID with a Copy button, with a label like a field's and help under it.
- A `Field` outside a form that shows the server's answer takes `state="invalid"` and the answer as its help; that is the only use of the `state` prop for a refusal.
