# Changelog

## [0.4.0](https://github.com/dantuck/vivo/compare/v0.3.2...v0.4.0) (2026-04-21)


### Features

* add RemoteBackend trait and B2Backend ([cb53540](https://github.com/dantuck/vivo/commit/cb53540d388d5ca08bcf8a2c17cf9e381498e886))
* add S3Backend for S3-compatible remotes via restic copy ([6bfce59](https://github.com/dantuck/vivo/commit/6bfce594c7a2a25a5846c6941cab71dfc72088c7))
* add Step enum for backup phase control ([3aecf5f](https://github.com/dantuck/vivo/commit/3aecf5f16feaa259c085829fbe382aa9fab5ba8b))
* doctor module with check functions for tools, config, secrets, and remotes ([d52944f](https://github.com/dantuck/vivo/commit/d52944f71d582c324d5e2ee78ab31fddd1eae1da))
* expose all_remotes() from BackupConfig for doctor connectivity checks ([cced1ba](https://github.com/dantuck/vivo/commit/cced1ba0a0498c01e6a2e0923c20ef4651fbf026))
* install.sh for one-line binary installation from github releases ([49ee453](https://github.com/dantuck/vivo/commit/49ee453120cfbd288f9891b716aa4495bdcbab0e))
* multi-remote backends, subcommands, secrets management, and quality fixes ([5745792](https://github.com/dantuck/vivo/commit/574579244e349232c86e0a0a899ca941c1b25abf))
* multi-remote backup with step gating, credential injection, calls/commands ([6cdeec1](https://github.com/dantuck/vivo/commit/6cdeec172a2fb514082d4d0243d3c61901b39845))
* self-update module with rate-limited version check and apply_update ([0fcbf28](https://github.com/dantuck/vivo/commit/0fcbf28f20f313e885b5d7297c024be70e49dcd0))
* split CLI build from arg parsing, add start_step and credentials to config ([c110344](https://github.com/dantuck/vivo/commit/c110344df56193940ed8bf4fed66aece174c8134))
* vivo doctor subcommand with structured health checks ([c00a4d0](https://github.com/dantuck/vivo/commit/c00a4d05f6b3d2b2e923133459f572c862a74a68))
* vivo update subcommand and periodic update check after backup ([237618e](https://github.com/dantuck/vivo/commit/237618e908b056342e7b796e0c5e5535c6efcf48))


### Bug Fixes

* enable thin LTO to work around Apple linker symbol-length assertion ([4759847](https://github.com/dantuck/vivo/commit/4759847ae6712dc1f51f5e6d22f0ae8c30f65a37))
* fall back to ~/.local/bin when install dir is not writable ([0019e31](https://github.com/dantuck/vivo/commit/0019e31e85db161e92e07ac3c8ae76b293b2e933))
* get_secrets_path ignores config_file arg; replace HOME.unwrap with expect ([aa5ffe5](https://github.com/dantuck/vivo/commit/aa5ffe53589ddf577796564913d493cbde7a0ea5))
* graceful execute_command error handling; fix sibling calls circular-ref detection ([b5a7f21](https://github.com/dantuck/vivo/commit/b5a7f214b184a5b4788f499bd93760ba552dbc0c))
* improve b2 error handling and use idiomatic path join ([ae5ef2d](https://github.com/dantuck/vivo/commit/ae5ef2d11237b329e1e6c0b10bee3f052915bb35))
* remove unused import in step tests ([631ed68](https://github.com/dantuck/vivo/commit/631ed681175357a7d9e6be9406e2b03d13ebce64))
* s3 private url field and accurate check_installed error message ([cf468e2](https://github.com/dantuck/vivo/commit/cf468e2eba855cef89d2a1e6f66adcad4e022eab))
* set tag-name to v${version} so releases use v0.x.x format ([9b939cc](https://github.com/dantuck/vivo/commit/9b939ccb5befcc50fde53e65108d17be2f92beb4))
* update github mirror url to dantuck/vivo ([e106db0](https://github.com/dantuck/vivo/commit/e106db013185e506180b8bd6c3112e645dd6fd68))
* update repository URL to GitHub primary remote ([d41921a](https://github.com/dantuck/vivo/commit/d41921ac8bb8717ebdda25adecf5ecf71d586f6b))
* use include-component-in-tag false so release tags use v${version} format ([c200892](https://github.com/dantuck/vivo/commit/c200892bfedbb01e713f2bf09038833a3544412d))
* use macos-13 (Xcode 15) to avoid Xcode 16 linker symbol-length bug ([081ef0b](https://github.com/dantuck/vivo/commit/081ef0bf2173afb6edcb0bb951ed1e60e6dd431e))
* use releases list endpoint to support pre-release versions ([2c1f260](https://github.com/dantuck/vivo/commit/2c1f260cb3e888ac45647edb755c4d85f3c760c4))

## [0.3.2](https://github.com/dantuck/vivo/compare/v0.3.1...v0.3.2) (2026-04-20)


### Bug Fixes

* fall back to ~/.local/bin when install dir is not writable ([0019e31](https://github.com/dantuck/vivo/commit/0019e31e85db161e92e07ac3c8ae76b293b2e933))

## [0.3.1](https://github.com/dantuck/vivo/compare/v0.3.0...v0.3.1) (2026-04-20)


### Bug Fixes

* update repository URL to GitHub primary remote ([d41921a](https://github.com/dantuck/vivo/commit/d41921ac8bb8717ebdda25adecf5ecf71d586f6b))
* use include-component-in-tag false so release tags use v${version} format ([c200892](https://github.com/dantuck/vivo/commit/c200892bfedbb01e713f2bf09038833a3544412d))

## [0.3.0](https://github.com/dantuck/vivo/compare/vivo-v0.2.0...vivo-v0.3.0) (2026-04-20)


### Features

* add RemoteBackend trait and B2Backend ([cb53540](https://github.com/dantuck/vivo/commit/cb53540d388d5ca08bcf8a2c17cf9e381498e886))
* add S3Backend for S3-compatible remotes via restic copy ([6bfce59](https://github.com/dantuck/vivo/commit/6bfce594c7a2a25a5846c6941cab71dfc72088c7))
* add Step enum for backup phase control ([3aecf5f](https://github.com/dantuck/vivo/commit/3aecf5f16feaa259c085829fbe382aa9fab5ba8b))
* doctor module with check functions for tools, config, secrets, and remotes ([d52944f](https://github.com/dantuck/vivo/commit/d52944f71d582c324d5e2ee78ab31fddd1eae1da))
* expose all_remotes() from BackupConfig for doctor connectivity checks ([cced1ba](https://github.com/dantuck/vivo/commit/cced1ba0a0498c01e6a2e0923c20ef4651fbf026))
* install.sh for one-line binary installation from github releases ([49ee453](https://github.com/dantuck/vivo/commit/49ee453120cfbd288f9891b716aa4495bdcbab0e))
* multi-remote backends, subcommands, secrets management, and quality fixes ([5745792](https://github.com/dantuck/vivo/commit/574579244e349232c86e0a0a899ca941c1b25abf))
* multi-remote backup with step gating, credential injection, calls/commands ([6cdeec1](https://github.com/dantuck/vivo/commit/6cdeec172a2fb514082d4d0243d3c61901b39845))
* self-update module with rate-limited version check and apply_update ([0fcbf28](https://github.com/dantuck/vivo/commit/0fcbf28f20f313e885b5d7297c024be70e49dcd0))
* split CLI build from arg parsing, add start_step and credentials to config ([c110344](https://github.com/dantuck/vivo/commit/c110344df56193940ed8bf4fed66aece174c8134))
* vivo doctor subcommand with structured health checks ([c00a4d0](https://github.com/dantuck/vivo/commit/c00a4d05f6b3d2b2e923133459f572c862a74a68))
* vivo update subcommand and periodic update check after backup ([237618e](https://github.com/dantuck/vivo/commit/237618e908b056342e7b796e0c5e5535c6efcf48))


### Bug Fixes

* enable thin LTO to work around Apple linker symbol-length assertion ([4759847](https://github.com/dantuck/vivo/commit/4759847ae6712dc1f51f5e6d22f0ae8c30f65a37))
* get_secrets_path ignores config_file arg; replace HOME.unwrap with expect ([aa5ffe5](https://github.com/dantuck/vivo/commit/aa5ffe53589ddf577796564913d493cbde7a0ea5))
* graceful execute_command error handling; fix sibling calls circular-ref detection ([b5a7f21](https://github.com/dantuck/vivo/commit/b5a7f214b184a5b4788f499bd93760ba552dbc0c))
* improve b2 error handling and use idiomatic path join ([ae5ef2d](https://github.com/dantuck/vivo/commit/ae5ef2d11237b329e1e6c0b10bee3f052915bb35))
* remove unused import in step tests ([631ed68](https://github.com/dantuck/vivo/commit/631ed681175357a7d9e6be9406e2b03d13ebce64))
* s3 private url field and accurate check_installed error message ([cf468e2](https://github.com/dantuck/vivo/commit/cf468e2eba855cef89d2a1e6f66adcad4e022eab))
* set tag-name to v${version} so releases use v0.x.x format ([9b939cc](https://github.com/dantuck/vivo/commit/9b939ccb5befcc50fde53e65108d17be2f92beb4))
* update github mirror url to dantuck/vivo ([e106db0](https://github.com/dantuck/vivo/commit/e106db013185e506180b8bd6c3112e645dd6fd68))
* use macos-13 (Xcode 15) to avoid Xcode 16 linker symbol-length bug ([081ef0b](https://github.com/dantuck/vivo/commit/081ef0bf2173afb6edcb0bb951ed1e60e6dd431e))
* use releases list endpoint to support pre-release versions ([2c1f260](https://github.com/dantuck/vivo/commit/2c1f260cb3e888ac45647edb755c4d85f3c760c4))

## [0.2.0](https://github.com/dantuck/vivo/compare/vivo-v0.1.0...vivo-v0.2.0) (2026-04-19)


### Features

* add RemoteBackend trait and B2Backend ([cb53540](https://github.com/dantuck/vivo/commit/cb53540d388d5ca08bcf8a2c17cf9e381498e886))
* add S3Backend for S3-compatible remotes via restic copy ([6bfce59](https://github.com/dantuck/vivo/commit/6bfce594c7a2a25a5846c6941cab71dfc72088c7))
* add Step enum for backup phase control ([3aecf5f](https://github.com/dantuck/vivo/commit/3aecf5f16feaa259c085829fbe382aa9fab5ba8b))
* doctor module with check functions for tools, config, secrets, and remotes ([d52944f](https://github.com/dantuck/vivo/commit/d52944f71d582c324d5e2ee78ab31fddd1eae1da))
* expose all_remotes() from BackupConfig for doctor connectivity checks ([cced1ba](https://github.com/dantuck/vivo/commit/cced1ba0a0498c01e6a2e0923c20ef4651fbf026))
* install.sh for one-line binary installation from github releases ([49ee453](https://github.com/dantuck/vivo/commit/49ee453120cfbd288f9891b716aa4495bdcbab0e))
* multi-remote backends, subcommands, secrets management, and quality fixes ([5745792](https://github.com/dantuck/vivo/commit/574579244e349232c86e0a0a899ca941c1b25abf))
* multi-remote backup with step gating, credential injection, calls/commands ([6cdeec1](https://github.com/dantuck/vivo/commit/6cdeec172a2fb514082d4d0243d3c61901b39845))
* self-update module with rate-limited version check and apply_update ([0fcbf28](https://github.com/dantuck/vivo/commit/0fcbf28f20f313e885b5d7297c024be70e49dcd0))
* split CLI build from arg parsing, add start_step and credentials to config ([c110344](https://github.com/dantuck/vivo/commit/c110344df56193940ed8bf4fed66aece174c8134))
* vivo doctor subcommand with structured health checks ([c00a4d0](https://github.com/dantuck/vivo/commit/c00a4d05f6b3d2b2e923133459f572c862a74a68))
* vivo update subcommand and periodic update check after backup ([237618e](https://github.com/dantuck/vivo/commit/237618e908b056342e7b796e0c5e5535c6efcf48))


### Bug Fixes

* get_secrets_path ignores config_file arg; replace HOME.unwrap with expect ([aa5ffe5](https://github.com/dantuck/vivo/commit/aa5ffe53589ddf577796564913d493cbde7a0ea5))
* graceful execute_command error handling; fix sibling calls circular-ref detection ([b5a7f21](https://github.com/dantuck/vivo/commit/b5a7f214b184a5b4788f499bd93760ba552dbc0c))
* improve b2 error handling and use idiomatic path join ([ae5ef2d](https://github.com/dantuck/vivo/commit/ae5ef2d11237b329e1e6c0b10bee3f052915bb35))
* remove unused import in step tests ([631ed68](https://github.com/dantuck/vivo/commit/631ed681175357a7d9e6be9406e2b03d13ebce64))
* s3 private url field and accurate check_installed error message ([cf468e2](https://github.com/dantuck/vivo/commit/cf468e2eba855cef89d2a1e6f66adcad4e022eab))
* update github mirror url to dantuck/vivo ([e106db0](https://github.com/dantuck/vivo/commit/e106db013185e506180b8bd6c3112e645dd6fd68))
