package io.github.zonnedev.jman.fixture.api;

/** Shared API used to verify transitive publication metadata. */
public final class Greeting {
  private Greeting() {}

  /** Returns the acceptance-test message fragment. */
  public static String message() {
    return "from published modules";
  }
}
