package dev.zonnedev.jman.example.grpc;

import io.micronaut.runtime.Micronaut;

public final class GrpcApplication {
  private GrpcApplication() {
  }

  public static void main(String[] arguments) {
    Micronaut.run(GrpcApplication.class, arguments);
  }
}
