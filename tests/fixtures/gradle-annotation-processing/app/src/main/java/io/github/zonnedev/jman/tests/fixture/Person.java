package io.github.zonnedev.jman.tests.fixture;

import lombok.Builder;
import lombok.Getter;
import lombok.Setter;
import lombok.extern.slf4j.Slf4j;

@Getter
@Setter
@Slf4j
@Builder
public final class Person {
  private String name;

  public String describe() {
    log.debug("describing {}", name);
    return this.getName();
  }

  public static Person create(String name) {
    return Person.builder().name(name).build();
  }
}
