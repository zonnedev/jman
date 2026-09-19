package io.github.zonnedev.jman.fixture.consumer;

import io.github.zonnedev.jman.fixture.library.Greeter;

public final class Application {
  private Application() {}

  public static void main(String[] arguments) {
    System.out.print(Greeter.greet());
  }
}
