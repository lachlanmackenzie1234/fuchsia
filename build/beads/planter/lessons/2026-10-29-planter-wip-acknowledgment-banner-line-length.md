# Planter wip acknowledgment banner line length exemption

- **Learned from:** inner-loop friction on CL I0071aec921ca31d2a6ab4ee3b1ad70ae9d58955b, patchset 4 (task `pkg-fake-channel-control`)
- **Date:** 2026-10-29
- **Changed:** `checks/commit_message_format.sh`, `checks/confidentiality_check.sh`

## Root cause

When planter uploads a WIP patchset awaiting uploader acknowledgment, it
prepends the 78-character banner
`[DO NOT SUBMIT/REVIEW] waiting on uploader acknowledgement of planter findings`
to the commit body and strips that banner during the post-acknowledgment
ready-for-review upload. When shared machinery advances while a CL is waiting
for acknowledgment, the post-acknowledgment pre-upload gate re-runs checks
before stripping the banner, causing `commit_message_format.sh` and
`confidentiality_check.sh` to flag line 3 for exceeding 72 characters and send
the acknowledged change back to `CODING`.

## Why not a one-off fix

Any WIP change awaiting acknowledgment whose machinery updates before re-upload
carries planter's synthetic 78-character banner in `HEAD` when the pre-upload
gate executes. Exempting planter's exact WIP acknowledgment banner in both
72-character body-line checks prevents unnecessary post-acknowledgment coder
and build round-trips across all tasks while keeping strict 72-character body
wrapping enforced on every real commit body line.
