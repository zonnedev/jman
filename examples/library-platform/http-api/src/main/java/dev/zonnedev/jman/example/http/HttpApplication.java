package dev.zonnedev.jman.example.http;

import io.micronaut.runtime.Micronaut;

public final class HttpApplication {
  private HttpApplication() {
  }

  public static void main(String[] arguments) {
    Micronaut.run(HttpApplication.class, arguments);
  }
}
