package dev.zonnedev.jman.example.client;

import java.net.URI;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class LibraryClientBuilderTest {
  @Test
  void requiresAnExplicitTransport() {
    Assertions.assertThrows(IllegalStateException.class, () -> LibraryClient.builder().build());
  }

  @Test
  void createsEitherTransportBehindTheSamePublicInterface() {
    try (
      var http = LibraryClient.builder()
      .http(URI.create("http://127.0.0.1:8080"))
      .build();
      var grpc = LibraryClient.builder().grpc("127.0.0.1", 9090).build()
    ) {
      Assertions.assertInstanceOf(LibraryClient.class, http);
      Assertions.assertInstanceOf(LibraryClient.class, grpc);
    }
  }
}
