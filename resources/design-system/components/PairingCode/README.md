# PairingCode

The six-digit code a client shows while it waits to pair, for a person to type on the server. The
same idea as Moonlight's PIN.

- Class `kp-paircode` holding `kp-paircode__digits`, the code as two groups of three in JetBrains Mono at 32px, on `bg-inset`.
- Put the instruction next to it, naming the server: "Type this code on lab-rack-3, under Clients, Waiting to pair."
- The code is never a link or a copy target: the point is that a person reads it and types it.
- The digits are hidden from screen readers, which get a `kp-sr-only` copy with the digits spaced out, so they are read one by one.
