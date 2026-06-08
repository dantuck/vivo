# Changelog

## [0.10.1](https://github.com/dantuck/vivo/compare/v0.10.0...v0.10.1) (2026-06-08)


### Bug Fixes

* **remote/rustfs:** auto-create bucket before sync if missing ([9c6b7ff](https://github.com/dantuck/vivo/commit/9c6b7ff6d9db1fdd41fc3e8bcca8f645dcc5bca5))

## [0.10.0](https://github.com/dantuck/vivo/compare/v0.9.0...v0.10.0) (2026-06-08)


### Features

* **doctor:** add --fix flag, update run_doctor signature ([5f660eb](https://github.com/dantuck/vivo/commit/5f660ebcd1c32d920666e5ea5950078a3423f94a))
* **doctor:** add fix_s3_sync_tool() with mc install and plain-doctor hint ([b2b681a](https://github.com/dantuck/vivo/commit/b2b681a54b7a9dafb9e3441594cce96627766308))
* **doctor:** add rustfs connectivity check via aws s3 ls ([f5c2a8e](https://github.com/dantuck/vivo/commit/f5c2a8ec9938aa13c09586e993bd214420acdba5))
* **doctor:** check mc/aws/rclone tool for s3/rustfs remotes ([ab2a514](https://github.com/dantuck/vivo/commit/ab2a514308e208fa2099da6497f54099c32cd91f))
* **remote:** add RustfsBackend with URL parsing ([41f88ce](https://github.com/dantuck/vivo/commit/41f88ce25225bafa32cf26197b30ec0bd4c55283))
* **remote:** add tool detection (mc/aws/rclone) to RustfsBackend ([dd90e56](https://github.com/dantuck/vivo/commit/dd90e565d0131388e153dbd977d5c66ee5b5b7fd))
* **remote:** implement RustfsBackend sync via mc/aws/rclone ([d45ba87](https://github.com/dantuck/vivo/commit/d45ba8709e62d791d64657a569314a3983627c47))
* **tui:** warn when adding s3/rustfs remote without mc/aws/rclone installed ([11de248](https://github.com/dantuck/vivo/commit/11de24837925ffdbae556fa868d9d62b9252e94d))


### Bug Fixes

* **doctor:** re-run full doctor after fix, use mc for rustfs connectivity check ([d5b3e79](https://github.com/dantuck/vivo/commit/d5b3e79b3d34506e52266998dfa243b167c5da4a))
* **remote:** pass credentials via env vars to avoid argv exposure in mc/rclone ([68d9f17](https://github.com/dantuck/vivo/commit/68d9f177aedc7103eabd291c6b49fa27e7668213))
* **remote:** stream S3Backend sync output to terminal ([950344e](https://github.com/dantuck/vivo/commit/950344e124e1ce6392041bf5a00bd9cb51de46ef))
* **remote:** verify repo before destructive sync, scope s3-tool check to rustfs only, make connectivity check tool-aware ([b8cf14e](https://github.com/dantuck/vivo/commit/b8cf14eccde89335cb6d487149e69bbe3ded8626))

## [0.9.0](https://github.com/dantuck/vivo/compare/v0.8.0...v0.9.0) (2026-06-06)


### Features

* add add_call/remove_call/move_call_up/down to config_editor ([1cffe64](https://github.com/dantuck/vivo/commit/1cffe6454f7beda94f068204c1d7689e227b32ba))
* add Pane::Calls, TaskEntry.calls, and App.selected_call ([d61bb53](https://github.com/dantuck/vivo/commit/d61bb5303fad6e97b22f8af61765455ddaabbda6))
* add s key to set default task in TUI ([2d2bc01](https://github.com/dantuck/vivo/commit/2d2bc01dc0426e7a41befb1e10d3ce8ad03522b3))
* add set_default_task to config_editor ([5627a2a](https://github.com/dantuck/vivo/commit/5627a2acaa5f4157cb33c2ea0f6d2a7392718de6))
* expose call_names() accessor on Task ([c2da3c3](https://github.com/dantuck/vivo/commit/c2da3c3a1b70c2aa631dad9eb1b859c0921fd34f))
* expose default_task field on App ([cf0f769](https://github.com/dantuck/vivo/commit/cf0f76944eb2d8c3468572ca81b80e94a36f8429))
* mark default task with * in task list ([3cd44e4](https://github.com/dantuck/vivo/commit/3cd44e4d2f4aef25ccb4900ee58782b3608047e9))
* render Calls section in TUI task detail pane ([e846149](https://github.com/dantuck/vivo/commit/e8461497b571f2440f2f1c34065f2134babdcd18))
* wire Calls pane events (add/delete/reorder, Tab cycle, navigation) ([b5bc92c](https://github.com/dantuck/vivo/commit/b5bc92cd618c6d18ddc188048c94f800c96a848c))


### Bug Fixes

* add confirmation prompt to delete_call_prompt to match Remotes behavior ([d9a4549](https://github.com/dantuck/vivo/commit/d9a4549fded2e4ade8d331d9ab2bc0e62e01fe51))
* make repo optional when adding a task, supporting calls-only tasks ([42ab511](https://github.com/dantuck/vivo/commit/42ab5117753a27532d008f57d4efb46383dd8e59))
* move_call_down propagates task-not-found error instead of silent no-op ([9171b6c](https://github.com/dantuck/vivo/commit/9171b6cfbefdfc38689993debbfd40d7103785c3))
* only update selected_call cursor when move operation succeeds ([b5cb7d9](https://github.com/dantuck/vivo/commit/b5cb7d9c6a7cab812516c2b21c1875de55f1cbbb))
* propagate error when default-task node has no value ([825c835](https://github.com/dantuck/vivo/commit/825c8353ea5038c8030a013c2284b615dc540e39))
* **tui:** size Remotes and Calls sections by content count ([97e82d1](https://github.com/dantuck/vivo/commit/97e82d14b28dd1ac3d7b1f38ed07103ed63d911f))

## [0.8.0](https://github.com/dantuck/vivo/compare/v0.7.0...v0.8.0) (2026-06-05)


### Features

* **backup_config:** add apply_profile_to_yaml and write_profile_to_secrets ([3df4754](https://github.com/dantuck/vivo/commit/3df47543afab7f59cbf96bc2086dfe21ab080b3b))
* **backup:** auto-init local restic repo before first backup ([d91bd23](https://github.com/dantuck/vivo/commit/d91bd2398739e2cd836ad9574009c25cda924193))
* **config:** add edit_remote ([7e91347](https://github.com/dantuck/vivo/commit/7e91347a4c4635ab06b7c1e4a8f6fa02958a8cfd))
* **config:** add edit_task with rename and reference updates ([a8eb527](https://github.com/dantuck/vivo/commit/a8eb52726d911754cf1dd4eaf9c8affb82701e0e))
* **doctor:** add check_fuse() and expose run_with_timeout; add ctrlc dep ([d3aaf20](https://github.com/dantuck/vivo/commit/d3aaf2036544040b5c49091bdd91400acc99735a))
* **mount:** add check_repo_accessible and run_preflight ([317af75](https://github.com/dantuck/vivo/commit/317af7560696245fd9230f1fe20c87b278a54cab))
* **mount:** add mount_point_path and check_mount_point_valid ([45a1537](https://github.com/dantuck/vivo/commit/45a15370d1cfe5d01cb0e922d56ef3b489e0ff67))
* **mount:** add MountEntry, build_entries, normalize_repo_url ([d1b4a73](https://github.com/dantuck/vivo/commit/d1b4a73b1d5ae87207f246f0988a57f5ce8e4a82))
* **mount:** implement run() with picker, preflight, restic mount, cleanup ([9b2ebd5](https://github.com/dantuck/vivo/commit/9b2ebd5b9885f955f6f2d8ef68124c28b1542d2f))
* **mount:** wire up vivo mount subcommand to CLI ([373e353](https://github.com/dantuck/vivo/commit/373e3530c3ae0273e9d6b8a9588c6cf385d05866))
* **tui:** add CredentialType enum and detect_url_type ([0a753af](https://github.com/dantuck/vivo/commit/0a753affc09553f154df17797d56af8b25640481))
* **tui:** add edit task/remote prompts, bind e and o keys ([11349b6](https://github.com/dantuck/vivo/commit/11349b684d7bd90fe4d2a7f75e72dcca31e2447a))
* **tui:** add fields pane with per-field selection and highlight ([b3c4b24](https://github.com/dantuck/vivo/commit/b3c4b244bd04925e4892f9061874eb2428def9dc))
* **tui:** add parse_profile_names and list_profiles ([21bb8e0](https://github.com/dantuck/vivo/commit/21bb8e01f1943d01ab2bab57120dea3c9aa7886c))
* **tui:** add select_or_create_profile interactive flow ([7e1cdf7](https://github.com/dantuck/vivo/commit/7e1cdf70dccbd28c62b3aaaf43a17d6c505e8a33))
* **tui:** add suggest_profile_name ([56f55b4](https://github.com/dantuck/vivo/commit/56f55b4e1d438316c55995a5c43457d826006ebe))
* **tui:** auto-create repo and directory paths when adding or editing tasks ([b515877](https://github.com/dantuck/vivo/commit/b5158772d1b6962a25e39250a174f2256850b499))
* **tui:** expose backup fields via public accessors ([a38c84b](https://github.com/dantuck/vivo/commit/a38c84bd31ed50a2877810466d0b06ca21622586))
* **tui:** extend TaskEntry with task detail fields ([29da393](https://github.com/dantuck/vivo/commit/29da3933b1d4c711e81be60b12786b70b67ae112))
* **tui:** offer restic init on edit-remote and test-remote when repo missing ([1ba57f3](https://github.com/dantuck/vivo/commit/1ba57f304e10f0ca6f0e490858fe0d8f3eb7e234))
* **tui:** offer restic init when adding a remote with no repo ([2d180c1](https://github.com/dantuck/vivo/commit/2d180c105e4f905043eb399074f0d221b3c91723))
* **tui:** replace credentials text prompt with profile select ([f393dc7](https://github.com/dantuck/vivo/commit/f393dc77b57d634ac6a17f44b058dda8e44d2180))
* **tui:** rewrite right pane as full task detail view ([d05a037](https://github.com/dantuck/vivo/commit/d05a037bde4eed9072b0cd0e915bcbc2aed862fd))


### Bug Fixes

* **backup_config:** use private 0600 temp file for secrets write ([7d9da6d](https://github.com/dantuck/vivo/commit/7d9da6da22b990149feb75dcee22aba73793200d))
* **config:** align edit_remote error message, add missing backup block test ([c228cae](https://github.com/dantuck/vivo/commit/c228caee4aa9f89bcf4ae8b856011e051bb51fbe))
* **config:** preserve node ordering in upsert_or_remove_child, repair malformed nodes ([8fb2610](https://github.com/dantuck/vivo/commit/8fb2610d9f250527783292b2a086f1adef3bf033))
* **doctor:** consistent FUSE label in fail branch; gate PATH test to Linux ([66174bc](https://github.com/dantuck/vivo/commit/66174bccd1ff9c6adae850fd8c9aece1c8c03d4a))
* **mount:** expand env vars in repo URL before preflight and mount ([85797eb](https://github.com/dantuck/vivo/commit/85797eb1bb1a5eb1b81c5f1c8dacb2dd675d190f))
* **mount:** lazy unmount on exit; hint to close files before Ctrl+C ([9ce0397](https://github.com/dantuck/vivo/commit/9ce0397d7378c748b11e5628d68b9489fa8aa934))
* **mount:** print unmount confirmation on exit ([b8411f8](https://github.com/dantuck/vivo/commit/b8411f85c1cba769a1b85e3e8e7376998105457a))
* **mount:** show snapshot browse paths after mounting ([a2aa443](https://github.com/dantuck/vivo/commit/a2aa44374926eb33d7fb6c87f3958d5cf9a5a766))
* **remote:** correct restic copy args and set source repo password for S3 sync ([0d73aa2](https://github.com/dantuck/vivo/commit/0d73aa2ba6c030f6aa317a34aea10dc601cbed3e))
* **tui:** clamp selected_remote index in render to guard against stale state ([1a262b8](https://github.com/dantuck/vivo/commit/1a262b81213a108ab77c8b2f32950f026e152fdf))
* **tui:** classify s3+https as S3, remove println from profile create ([679ed9e](https://github.com/dantuck/vivo/commit/679ed9e931257d263d56b044d15b2a2cebc39f3e))
* **tui:** force full repaint after suspend/resume to fix stale screen on prompt return ([d8123e7](https://github.com/dantuck/vivo/commit/d8123e7c20aa271f6f1238ead26f8ec4da082361))
* **tui:** ignore e/d in Remotes pane when task has no remotes ([98f9dc8](https://github.com/dantuck/vivo/commit/98f9dc8607481d1bd9dffd24da8bcfdee2c958d7))
* **tui:** preserve status message and pane focus across reload; validate remote URL ([10015a8](https://github.com/dantuck/vivo/commit/10015a85f70a29758d2b1bb07b005453f705c169))
* **tui:** remove s3+https from detect_url_type (backend unsupported) ([f85b28f](https://github.com/dantuck/vivo/commit/f85b28fcd2839170bdd7e69144480f9fc79d8167))
* **tui:** translate rustfs: to s3: before passing URL to restic in test-remote ([7e52cd2](https://github.com/dantuck/vivo/commit/7e52cd2252f0b950978f5ebf441e0f884bd4ab24))
* **tui:** treat prompt cancel (Esc/Ctrl-C) as silent no-op instead of error ([12a77f3](https://github.com/dantuck/vivo/commit/12a77f34694d4372f2a4c48038ce8ff7fdba64b8))
* **tui:** trim profile name before duplicate check and save ([aa26e09](https://github.com/dantuck/vivo/commit/aa26e09c752941227180187d86e2f06b4918f6c9))

## [0.7.0](https://github.com/dantuck/vivo/compare/v0.6.0...v0.7.0) (2026-06-04)


### Features

* add update_s3_in_secrets, expose backup_remotes and description on Task ([93bbcd7](https://github.com/dantuck/vivo/commit/93bbcd79fcfb4ac87a5cd85438fc7826a01ea7f1))
* **cli:** add vivo remote add/list/remove subcommands ([b01702b](https://github.com/dantuck/vivo/commit/b01702b0ce6a5634a21360803e0143fe46b50b75))
* **cli:** add vivo secrets import-s3 and update CONFIG_TEMPLATE with rustfs example ([4a4220b](https://github.com/dantuck/vivo/commit/4a4220bb423a0ac04892682df47426ecc89c0c98))
* **cli:** add vivo task add/list/remove subcommands ([871571e](https://github.com/dantuck/vivo/commit/871571e488e4e4a2006e9a7c4ac399a85671dbe1))
* **config-editor:** add TaskSpec, RemoteSpec, and add_task ([44b5d81](https://github.com/dantuck/vivo/commit/44b5d81d364700b92e99eb40f0ad60b21f2c1dce))
* **config-editor:** implement add_remote ([2c76e07](https://github.com/dantuck/vivo/commit/2c76e07ec44849987e566d2a32761d0e5076157a))
* **config-editor:** implement remove_remote and re-export all editor functions ([6f26a7a](https://github.com/dantuck/vivo/commit/6f26a7a5dab51c669f1ce6bae7529fdea4c973c5))
* **config-editor:** implement remove_task ([fd10c20](https://github.com/dantuck/vivo/commit/fd10c20c88bae0f07b8d2ff3b548a968b1248096))
* **remote:** add rustfs: URL prefix alias for S3-compatible remotes ([f858874](https://github.com/dantuck/vivo/commit/f8588746cb478b23fe31a993ed4beb42432b95ff))
* **tui:** add vivo manage skeleton with stub render and quit key ([37694ea](https://github.com/dantuck/vivo/commit/37694eae5dda6ec0be5505f0e19651782d887a99))
* **tui:** implement full event handling — navigate, add, delete, edit in vivo manage ([06dfc85](https://github.com/dantuck/vivo/commit/06dfc85022caab5f3d7411e2034aaa0b0d9efd0a))
* **tui:** implement two-pane layout with task list and remote detail ([51e451b](https://github.com/dantuck/vivo/commit/51e451b3537cc44f710a4d93024300e2551d718d))


### Bug Fixes

* **tui:** reload app state after editing config in $EDITOR ([5603004](https://github.com/dantuck/vivo/commit/5603004680616e5488fa8bc9f52aae8ed965cc1d))

## [0.6.0](https://github.com/dantuck/vivo/compare/v0.5.0...v0.6.0) (2026-04-24)


### Features

* add dot-matrix banner to --help, help, and init commands ([8e0976b](https://github.com/dantuck/vivo/commit/8e0976b86b0e2ac698d6e2cd75f98e880c95d581))

## [0.5.0](https://github.com/dantuck/vivo/compare/v0.4.5...v0.5.0) (2026-04-23)


### Features

* add B2 credential import with automatic re-auth on backup failure ([92028c2](https://github.com/dantuck/vivo/commit/92028c242aab660946e235941ea60d80a1de0c5e))

## [0.4.5](https://github.com/dantuck/vivo/compare/v0.4.4...v0.4.5) (2026-04-22)


### Bug Fixes

* pass age recipient key to sops during secrets init ([983f7ad](https://github.com/dantuck/vivo/commit/983f7ade590c9964ce8a94411d1e03f74b534d27))

## [0.4.4](https://github.com/dantuck/vivo/compare/v0.4.3...v0.4.4) (2026-04-22)


### Bug Fixes

* remove mac builds ([32e1094](https://github.com/dantuck/vivo/commit/32e1094a7e8396b53df5e22bccd976518d960c50))

## [0.4.3](https://github.com/dantuck/vivo/compare/v0.4.2...v0.4.3) (2026-04-21)


### Bug Fixes

* probe asset URL to skip releases whose binaries are not yet published ([18dd901](https://github.com/dantuck/vivo/commit/18dd901085ef0dea915899a22f7c2b4dcda9a959))

## [0.4.2](https://github.com/dantuck/vivo/compare/v0.4.1...v0.4.2) (2026-04-21)


### Bug Fixes

* skip releases with no assets when finding latest version ([22e7295](https://github.com/dantuck/vivo/commit/22e7295fffebedbfe5d216eeef330b97a561ad11))

## [0.4.1](https://github.com/dantuck/vivo/compare/v0.4.0...v0.4.1) (2026-04-21)


### Bug Fixes

* pass --repo to gh workflow run so it works without a checkout ([5f83063](https://github.com/dantuck/vivo/commit/5f830631415b7b4c3ac651de55ca94de5dd5b226))

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
