package dev.example.protobuf;

import dev.example.protobuf.model.GreetingProtocol;

public final class Application {
  private Application() {
  }

  public static void main(String[] args) {
    var greeting = GreetingProtocol.Greeting.newBuilder()
      .setMessage("Hello from generated Protobuf code!")
      .build();
    System.out.println(greeting.getMessage());
  }
}

