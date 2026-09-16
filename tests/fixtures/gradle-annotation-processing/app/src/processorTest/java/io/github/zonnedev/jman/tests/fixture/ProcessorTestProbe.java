package io.github.zonnedev.jman.tests.fixture;

@GenerateGreeting
final class ProcessorTestProbe {
  String greeting() {
    return GeneratedGreeting.message();
  }
}
