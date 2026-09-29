package dev.zonnedev.jman.example.client;

import com.sun.net.httpserver.HttpServer;
import dev.zonnedev.jman.example.contract.grpc.LibraryGrpcContract;
import dev.zonnedev.jman.example.contract.grpc.LibraryProto;
import io.grpc.ServerBuilder;
import io.grpc.ServerServiceDefinition;
import io.grpc.stub.ServerCalls;
import java.net.InetSocketAddress;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class ClientTransportIntegrationTest {
  @Test
  void httpTransportImplementsThePublicClientContract() throws Exception {
    var server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
    server.setExecutor(Executors.newVirtualThreadPerTaskExecutor());
    server.createContext(
      "/users",
      exchange -> {
        var body = "{\"id\":\"user-1\",\"name\":\"Ada\",\"bookIds\":[]}".getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().add("content-type", "application/json");
        exchange.sendResponseHeaders(201, body.length);
        try (var response = exchange.getResponseBody()) {
          response.write(body);
        }
      }
    );
    server.start();
    try (var client = LibraryClient.builder()
      .http(URI.create("http://127.0.0.1:" + server.getAddress().getPort()))
      .build()) {
      Assertions.assertEquals("Ada", client.createUser("Ada").name());
    } finally {
      server.stop(0);
    }
  }

  @Test
  void grpcTransportImplementsTheSamePublicClientContract() throws Exception {
    var definition = ServerServiceDefinition.builder(LibraryGrpcContract.SERVICE)
      .addMethod(
      LibraryGrpcContract.CREATE_USER,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        observer.onNext(LibraryProto.UserResponse.newBuilder()
          .setId("user-1")
          .setName(request.getName())
          .build());
        observer.onCompleted();
      }
    )
    )
      .build();
    var server = ServerBuilder.forPort(0).addService(definition).build().start();
    try (var client = LibraryClient.builder()
      .grpc("127.0.0.1", server.getPort())
      .build()) {
      Assertions.assertEquals("Ada", client.createUser("Ada").name());
    } finally {
      server.shutdownNow().awaitTermination(5, TimeUnit.SECONDS);
    }
  }
}
