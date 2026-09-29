package dev.zonnedev.jman.example.domain;

import java.util.UUID;

public record BookId(UUID value) {
  public BookId {
    if (value == null) {
      throw new IllegalArgumentException("book id is required");
    }
  }

  public static BookId create() {
    return new BookId(UUID.randomUUID());
  }

  public static BookId parse(String value) {
    return new BookId(UUID.fromString(value));
  }
}
