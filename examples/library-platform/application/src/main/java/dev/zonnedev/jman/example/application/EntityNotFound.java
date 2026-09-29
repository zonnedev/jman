package dev.zonnedev.jman.example.application;

public final class EntityNotFound extends RuntimeException {
  public EntityNotFound(String message) {
    super(message);
  }
}
