/** A time of day in the person's own format, to the minute: "10:15 AM", "14:05". The one spelling for every absolute time. */
export const clock = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });

/** A calendar day: "12 Nov 2026". */
export const date = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
