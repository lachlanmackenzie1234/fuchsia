# Migration Package Selection

## 1. Find a package

Use the "gsheets" skill: In the **control table**, find the first row from the top where the "Migration Status" column is "ready_for_migration" and the "Owner" column is empty.
   * If no row meets the requirements, fail the Migration Package Selection step with the reason: "No migration package is found."
   * The row that meets the requirements is the **migration package**.


## 2. Claim migration ownership

For the **migration package**, update the "Owner" column to the current user to claim ownership.

Wait for 5 seconds and then re-check the row whether the "Owner" column is the current user.
   * If not, refer to [Find a package](#1-find-a-package) to find another package for the migration.


## 3. Check the Buganizer Ticket

For the **migration package**, if the "Buganizer Ticket" column is empty, refer to [Buganizer ticket template](../assets/buganizer_ticket_template.md) to create a Buganizer ticket for this migration with "buganizer-cli" agent skill and write it to the "Buganizer Ticket" column.
   * If it fails to create the ticket, use the "Parent ID" defined in the [Buganizer ticket template](../assets/buganizer_ticket_template.md) as the Buganizer ticket number and write to the "Buganizer Ticket" column.

## 4. Update Control Table

After the ownership of the **migration package** is claimed, change "Migration Status" to "in-progress".
