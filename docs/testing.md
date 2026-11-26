# Testing Nimbra

```sh
cargo test
```

runs every unit test. They use sample data and `example.com` addresses, and never touch the network or a real
mailbox.

## Live tests

The IMAP and SMTP code is in [nimbra-core](https://github.com/nimbra-mail/core) now, with its live tests against
local Dovecot and Mailpit servers; its `docs/testing.md` has the setup.
