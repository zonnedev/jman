package dev.zonnedev.jman.example.domain;

public record BookTitle(String value) {
  public BookTitle {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException("book title is required");
    }
    value = value.strip();
  }
}
