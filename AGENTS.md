# Commit policy

Commit messages are in English and end with a trailer block typed as plain
message text:

```text
Signed-off-by: <your own name> <your own email>
```

The `Signed-off-by` must be the identity of the account making the commit
(for the maintainer: `Signed-off-by: p1xelll <81173399+p1xelll@users.noreply.github.com>`).
Never hardcode a fixed shared name or email, so other contributors' commits
are never signed with someone else's identity.

If and only if an AI tool actually co-authored the commit, add its trailer
too, e.g. `Co-Authored-By: Mistral Vibe <vibe@mistral.ai>` for commits
co-authored by Mistral Vibe. Do not add that trailer to commits made without
it.

Do not use `git commit -s`.
