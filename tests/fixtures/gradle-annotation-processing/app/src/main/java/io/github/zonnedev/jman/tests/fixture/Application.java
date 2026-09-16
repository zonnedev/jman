package io.github.zonnedev.jman.tests.fixture;

@GenerateGreeting
public final class Application {
  public String greeting() {
    return GeneratedGreeting.message();
  }
}

