package dev.zonnedev.jman.example.client;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.ObjectMapper;
import dev.zonnedev.jman.example.contract.http.LibraryHttpContract;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.util.List;

final class HttpLibraryClient implements LibraryClient {
  private final URI endpoint;
  private final HttpClient client = HttpClient.newHttpClient();
  private final ObjectMapper json = new ObjectMapper().findAndRegisterModules();

  HttpLibraryClient(URI endpoint) {
    this.endpoint = endpoint;
  }

  @Override
  public ClientUser createUser(String name) {
    var request = new LibraryHttpContract.CreateUserRequest(name);
    return user(send("POST", "/users", request, LibraryHttpContract.UserResponse.class));
  }

  @Override
  public List<ClientUser> listUsers() {
    return sendList("GET", "/users", null, new TypeReference<List<LibraryHttpContract.UserResponse>>() {
    })
      .stream()
      .map(HttpLibraryClient::user)
      .toList();
  }

  @Override
  public ClientBook registerBook(String ownerId, String title) {
    var request = new LibraryHttpContract.RegisterBookRequest(title);
    return book(send("POST", "/users/" + ownerId + "/books", request, LibraryHttpContract.BookResponse.class));
  }

  @Override
  public List<ClientBook> listBooks(String ownerId) {
    return sendList("GET", "/users/" + ownerId + "/books", null, new TypeReference<List<LibraryHttpContract.BookResponse>>() {
    })
      .stream()
      .map(HttpLibraryClient::book)
      .toList();
  }

  @Override
  public ClientBook transferBook(String bookId, String newOwnerId) {
    var request = new LibraryHttpContract.TransferBookRequest(newOwnerId);
    return book(send("PUT", "/books/" + bookId + "/owner", request, LibraryHttpContract.BookResponse.class));
  }

  @Override
  public void close() {
  }

  private <T> T send(
    String method,
    String path,
    Object body,
    Class<T> responseType
  ) {
    try {
      var request = request(method, path, body);
      var response = client.send(request, HttpResponse.BodyHandlers.ofString());
      requireSuccess(response);
      return json.readValue(response.body(), responseType);
    } catch (IOException exception) {
      throw new LibraryClientException("HTTP request failed", exception);
    } catch (InterruptedException exception) {
      Thread.currentThread().interrupt();
      throw new LibraryClientException("HTTP request was interrupted", exception);
    }
  }

  private <T> T sendList(
    String method,
    String path,
    Object body,
    TypeReference<T> responseType
  ) {
    try {
      var response = client.send(request(method, path, body), HttpResponse.BodyHandlers.ofString());
      requireSuccess(response);
      return json.readValue(response.body(), responseType);
    } catch (IOException exception) {
      throw new LibraryClientException("HTTP request failed", exception);
    } catch (InterruptedException exception) {
      Thread.currentThread().interrupt();
      throw new LibraryClientException("HTTP request was interrupted", exception);
    }
  }

  private HttpRequest request(String method, String path, Object body) throws IOException {
    var builder = HttpRequest.newBuilder(endpoint.resolve(path))
      .header("accept", "application/json");
    if (body == null) {
      return builder.method(method, HttpRequest.BodyPublishers.noBody()).build();
    }
    return builder.header("content-type", "application/json")
      .method(method, HttpRequest.BodyPublishers.ofString(json.writeValueAsString(body)))
      .build();
  }

  private static void requireSuccess(HttpResponse<String> response) {
    if (response.statusCode() < 200 || response.statusCode() >= 300) {
      throw new LibraryClientException("HTTP " + response.statusCode() + ": " + response.body());
    }
  }

  private static ClientUser user(LibraryHttpContract.UserResponse response) {
    return new ClientUser(response.id(), response.name(), response.bookIds());
  }

  private static ClientBook book(LibraryHttpContract.BookResponse response) {
    return new ClientBook(response.id(), response.title(), response.ownerId());
  }
}
