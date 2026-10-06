# Error Handling

This document handles error conditions (e.g., STOP). For non-error conditions, refer to the notifications.md file.

## Common Error Handling Steps For All Error Conditions
* Use "gmail" skill: refer to the "Interrupted Notification Template" in the [Notification Templates](../assets/notification_templates.md) to draft the "Interrupted" notification and send it to the user.
* Use the "gsheets" skill: in the **control table**, change "Migration Status" to "blocked".


## Error Handling Steps specific to the "STOP and CLEANUP" Condition
* Common error handling steps.
* Use `git` command to revert the changes.
