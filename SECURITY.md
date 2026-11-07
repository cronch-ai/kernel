# Security

Report a vulnerability privately, through GitHub: **Security → Report a vulnerability** on this repo. Please don't open a public issue for it.

## What counts

The kernel's job is evidence that anyone can re-check, so a vulnerability is anything that makes evidence say something false:

- A verdict, or a proof the checkers accept, for a property or rule that doesn't hold.
- A rule's queries or Lean statements that don't say what the rule says.
- A run that does what its capabilities' contracts rule out: a call whose precondition fails, a grant widened by anything but a trusted result or message, or a child that exceeds its parent.
- A value that loses a label it should carry.

If you're not sure whether something counts, report it privately anyway.
