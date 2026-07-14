# Moderation semantics

## Suspension vs ban

| Action | `is_active` | Sessions | After lift |
|--------|-------------|----------|------------|
| **Suspension** | Unchanged (stays true) | Revoked | Extractor allows access again once the moderation row is inactive (user may need to log in again) |
| **Ban** | Set **false** | Revoked | Lift clears the ban row only; member stays **inactive** until an officer/admin sets `is_active: true` |

Ban is “remove from the org.” Lift is “no longer banned,” not “restore membership.” Re-activation is an explicit second step so re-onboarding is intentional.

## Last actionable admin

An **actionable admin** is `org_role = admin`, `is_active = true`, and not currently suspended/banned. Last-admin guards (demote, deactivate, suspend/ban) and first-boot setup (`has_admin_member`) use this count so suspended admins cannot inflate protection and lock the org out.
