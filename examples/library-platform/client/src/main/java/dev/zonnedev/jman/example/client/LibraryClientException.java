package dev.zonnedev.jman.example.client;

public final class LibraryClientException extends RuntimeException {
  public LibraryClientException(String message) {
    super(message);
  }

  public LibraryClientException(String message, Throwable cause) {
    super(message, cause);
  }
}
