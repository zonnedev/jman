package dev.zonnedev.jman.example.domain;

public final class DomainConflict extends RuntimeException {
  public DomainConflict(String message) {
    super(message);
  }
}
