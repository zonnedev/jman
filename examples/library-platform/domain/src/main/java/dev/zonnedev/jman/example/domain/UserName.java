package dev.zonnedev.jman.example.domain;

public record UserName(String value) {
  public UserName {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException("user name is required");
    }
    value = value.strip();
  }
}
