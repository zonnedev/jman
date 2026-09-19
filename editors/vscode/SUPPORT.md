# JMAN Java support

## Before opening an issue

1. Update to the latest JMAN Java release.
2. Run **JMAN Java: Show Status** from the Command Palette.
3. Check **View → Output → JMAN Java** for startup, synchronization, or protocol
   errors.
4. Confirm that `jman.java.javaHome` points to Java 25 GraalVM and that the
   selected Maven or Gradle wrapper runs successfully from a terminal.
5. Reproduce the issue with other Java language-server extensions disabled for
   the workspace.

## Bug reports and feature requests

Use [GitHub Issues](https://github.com/zonnedev/jman/issues). Include:

- JMAN Java and VS Code versions;
- Linux distribution and CPU architecture;
- selected build system and synchronization mode;
- minimal reproduction steps;
- sanitized JMAN Java output-channel logs.

Do not attach credentials, repository tokens, private source code, complete
environment dumps, or unsanitized build logs.

## Security reports

Do not disclose suspected vulnerabilities in a public issue. Use GitHub's
private vulnerability reporting for the
[zonnedev/jman repository](https://github.com/zonnedev/jman/security) when it is
available. If that channel is unavailable, contact the publisher privately
through the Visual Studio Marketplace publisher profile before sharing details.

Supported versions and the coordinated-disclosure process are documented in
the repository's [security policy](https://github.com/zonnedev/jman/security/policy).
