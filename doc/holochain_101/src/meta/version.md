# Version
You are able to call `hdk::version()` on the hdk and it is able to tell you which version of the hdk you are using. This information is derived from the last tag that was applied to the holochain release and corresponds with our releases. The basis of this information comes from the `version` entry in the Cargo.toml of holochain Conductor, which is configured to track the release tag.