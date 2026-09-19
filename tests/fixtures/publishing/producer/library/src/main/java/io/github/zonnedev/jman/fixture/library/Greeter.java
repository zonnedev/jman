package io.github.zonnedev.jman.fixture.library;

import io.github.zonnedev.jman.fixture.api.Greeting;

/** Public facade consumed by each independent build tool. */
public final class Greeter {
  private Greeter() {}

  /** Returns a message whose implementation requires the transitive API JAR. */
  public static String greet() {
    return "hello " + Greeting.message();
  }
}
