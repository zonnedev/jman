package dev.zonnedev.jman.example.http;

import io.micronaut.context.ApplicationContext;
import io.micronaut.runtime.server.EmbeddedServer;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.Map;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class HttpApiIntegrationTest {
  @Test
  void servesTheGeneratedOpenApiRouteOverARealSocket() throws Exception {
    try (
      var context = ApplicationContext.run(Map.of("micronaut.server.port", -1));
      var server = context.getBean(EmbeddedServer.class).start()
    ) {
      var request = HttpRequest.newBuilder(URI.create(server.getURI() + "/users"))
        .header("content-type", "application/json")
        .POST(HttpRequest.BodyPublishers.ofString("{\"name\":\"Ada\"}"))
        .build();
      var response = HttpClient.newHttpClient().send(request, HttpResponse.BodyHandlers.ofString());

      Assertions.assertEquals(201, response.statusCode(), response.body());
      Assertions.assertTrue(response.body().contains("Ada"));
    }
  }
}
