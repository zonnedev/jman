package io.github.zonnedev.jman.javac;

/** Focused conformance tests for JJFS comment normalization and prose safety. */
final class JjfsCommentsTest {
  private JjfsCommentsTest() {
  }

  static void run() {
    canonicalizesJavadocStructureAndTags();
    wrapsOnlyStandalonePlainLineComments();
    preservesStructuredAndInlineComments();
  }

  private static void canonicalizesJavadocStructureAndTags() {
    String formatted =
        JjfsComments.format(
            """
            /**
            * Loads the customer information from the configured repository and validates that the customer can participate in the requested operation without changing the authored meaning.
            *
            * @param identifier the external customer identifier used to locate the customer in the configured repository
            * @return the matching active customer when it can participate in the operation
            */
            """.stripTrailing(),
            "JAVADOC_BLOCK",
            1,
            false);

    assertEquals("/**", formatted.lines().findFirst().orElseThrow());
    assertTrue(formatted.contains("\n   * Loads the customer information"), formatted);
    assertTrue(formatted.contains("\n   *\n   * @param identifier"), formatted);
    assertTrue(formatted.endsWith("\n   */"), formatted);
    assertTrue(formatted.lines().allMatch(line -> line.length() <= 100), formatted);
    assertWordsPreserved(
        "Loads the customer information from the configured repository and validates that the customer can participate in the requested operation without changing the authored meaning.",
        formatted.substring(formatted.indexOf("Loads"), formatted.indexOf("@param")));
    assertEquals(formatted, JjfsComments.format(formatted, "JAVADOC_BLOCK", 1, false));
  }

  private static void wrapsOnlyStandalonePlainLineComments() {
    String comment =
        "// Explains why the customer must be validated before the repository operation is started and before any external event can be published.";
    String formatted = JjfsComments.format(comment, "LINE", 2, false);
    assertTrue(formatted.contains("\n    // "), formatted);
    assertTrue(formatted.lines().allMatch(line -> line.length() <= 100), formatted);
    assertEquals(comment, JjfsComments.format(comment, "LINE", 2, true));
    String markdown =
        "/// A deliberately long Markdown documentation heading that must remain one authored line because the following line may be a Setext underline.";
    assertEquals(markdown, JjfsComments.format(markdown, "JAVADOC_LINE", 0, false));
  }

  private static void preservesStructuredAndInlineComments() {
    String structured =
        """
        /**
         * Supported transitions:
         *
         * - CREATED -> ACTIVE
         * - ACTIVE -> SUSPENDED
         *
         * <pre>
         * Customer
         *   └── PaymentMethod
         * </pre>
         *
         * See https://example.com/customer-transitions
         */
        """.stripTrailing();
    String formatted = JjfsComments.format(structured, "JAVADOC_BLOCK", 0, false);
    assertTrue(formatted.contains(" * - CREATED -> ACTIVE"), formatted);
    assertTrue(formatted.contains(" *   └── PaymentMethod"), formatted);
    assertTrue(formatted.contains(" * See https://example.com/customer-transitions"), formatted);
    assertEquals("/* keep   authored spacing */", JjfsComments.format("/* keep   authored spacing */", "BLOCK", 1, true));
    assertEquals("// jjfs: off", JjfsComments.format("// jjfs: off", "LINE", 1, false));
    String license =
        "/**\n * Copyright 2026 Example\n * Licensed under a license with deliberately   authored spacing.\n */";
    assertEquals(license, JjfsComments.format(license, "JAVADOC_BLOCK", 0, false));
  }

  private static void assertWordsPreserved(String expected, String formattedSection) {
    String actual =
        formattedSection
            .replace("*", " ")
            .replaceAll("\\s+", " ")
            .strip();
    assertEquals(expected, actual);
  }

  private static void assertTrue(boolean condition, String message) {
    if (!condition) throw new AssertionError(message);
  }

  private static void assertEquals(String expected, String actual) {
    if (!expected.equals(actual)) {
      throw new AssertionError("expected <%s> but was <%s>".formatted(expected, actual));
    }
  }
}
