package dev.zonnedev.jman.example.client;

import java.net.URI;

public final class LibraryClientBuilder {
  private Transport transport;
  private URI endpoint;
  private String host;
  private int port;

  public LibraryClientBuilder http(URI endpoint) {
    this.transport = Transport.HTTP;
    this.endpoint = endpoint;
    return this;
  }

  public LibraryClientBuilder grpc(String host, int port) {
    this.transport = Transport.GRPC;
    this.host = host;
    this.port = port;
    return this;
  }

  public LibraryClient build() {
    if (transport == null) {
      throw new IllegalStateException("select HTTP or gRPC before building the client");
    }
    return switch (transport) {
      case HTTP -> new HttpLibraryClient(requiredEndpoint());
      case GRPC -> new GrpcLibraryClient(requiredHost(), requiredPort());
    };
  }

  private URI requiredEndpoint() {
    if (endpoint == null || !endpoint.isAbsolute()) {
      throw new IllegalStateException("an absolute HTTP endpoint is required");
    }
    return endpoint;
  }

  private String requiredHost() {
    if (host == null || host.isBlank()) {
      throw new IllegalStateException("a gRPC host is required");
    }
    return host;
  }

  private int requiredPort() {
    if (port < 1 || port > 65_535) {
      throw new IllegalStateException("a valid gRPC port is required");
    }
    return port;
  }

  private enum Transport {
    HTTP,
    GRPC,
  }
}
