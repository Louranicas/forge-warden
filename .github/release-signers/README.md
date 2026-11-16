# Release signers

`allowed_signers` lists the keys whose signature on a `v<version>` tag `scripts/release_tag.sh`
accepts. The script reads this file from the commit it tags, never from a working tree, and deletes a
tag signed by any other key before anything is pushed. The file starts with no signer: the owner
enrols the release signing key's public half (`principal namespaces="git" ssh-ed25519 KEY`) in a
separately reviewed change. The private key stays with the owner.
