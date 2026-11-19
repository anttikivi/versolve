# Versolve

`versolve` is a small utility that can automatically resolve unique version
numbers that follow [semantic versioning](https://semver.org/) for each source
control revision in a Git repository. It uses tags for resolving the version
numbers, and the version numbers are constructed and formatted in a way that is
correctly ordered according to the versioning specification. It can use the tag
for version number, and when the revision is "between" tags, it uses the
commit's distance from the tag and it follows these rules to resolve the
version:

- No version tags: `0.1.0-0.<distance>.<branch>`.
- Last tag is a pre-release version: `<tag version>.0.<distance>.<branch>`.
- Last tag is not a pre-release version:
  `<major>.<minor>.<patch + 1>-0.<distance>.<branch>`.

You can set a default branch to Versolve, in which case the conflict-resolving
`<branch>` identifier is omitted when the version number is resolved for the set
default branch.

## License

© 2026 Antti Kivi

Versolve is licensed under the
[GNU General Public License v3.0 or later](https://www.gnu.org/licenses/).
