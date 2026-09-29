package dev.zonnedev.jman.example.domain;

import java.util.UUID;

public record UserId(UUID value) {
  public UserId {
    if (value == null) {
      throw new IllegalArgumentException("user id is required");
    }
  }

  public static UserId create() {
    return new UserId(UUID.randomUUID());
  }

  public static UserId parse(String value) {
    return new UserId(UUID.fromString(value));
  }
}
