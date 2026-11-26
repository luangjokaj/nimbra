# Nimbra

Project rules learned the hard way live in `tasks/lessons.md`. Read them before changing anything.

## Commit messages

Follow [Conventional Commits v1.0.0](https://www.conventionalcommits.org/en/v1.0.0/) strictly.

- Format: `<type>[optional scope]: <description>`, with optional body and footer sections for complex changes.
- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
- The description is lowercase, in the imperative mood, with no period at the end.
- Use a scope when it clarifies the area of change, for example `feat(auth): add jwt token validation`.
- For breaking changes, add `!` after the type or scope and include `BREAKING CHANGE:` in the footer.
- When a commit has several or significant changes, the body describes all of them professionally and in detail.
