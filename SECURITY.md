# Security policy

## Supported versions

Stackstead is pre-1.0. Security fixes are applied to the latest published minor
release and to the default branch. Older pre-1.0 minors may be asked to upgrade
before receiving a fix.

## Report a vulnerability privately

Use the repository's **Report a vulnerability** button to open a private GitHub
Security Advisory. Include the affected version or commit, operating system,
runtime provider, reproduction steps, impact, and any proposed mitigation.

Do not open a public issue for an undisclosed vulnerability. If private
reporting is unavailable, contact the maintainers through a private channel
listed on the repository profile and withhold exploit details until a private
channel is established.

We will acknowledge a complete report, reproduce it, assess affected versions,
and coordinate disclosure and credit with the reporter. Response timing depends
on severity and maintainer availability; this project does not promise a paid
bounty or a fixed service-level agreement.

## Security boundary

Stackstead prevents accidental runtime-identity collisions and wrong-target
cleanup. It is not a hostile-code sandbox, secret manager, multi-user
authorization boundary, or Docker-daemon isolation layer. A process launched by
Stackstead has the permissions of the invoking user. External volumes, globally
named volumes, bind mounts, host networking, and services outside the configured
runtime can still share state.
