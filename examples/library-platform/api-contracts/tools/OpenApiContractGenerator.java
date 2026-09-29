import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

final class OpenApiContractGenerator {
  private static final List<String> REQUIRED_OPERATIONS = List.of("createUser", "getUser", "listUsers", "registerBook", "listBooks", "transferBook");

  public static void main(String[] arguments) throws Exception {
    if (arguments.length != 2) {
      throw new IllegalArgumentException("expected OPENAPI_FILE OUTPUT_DIRECTORY");
    }
    var source = Files.readString(Path.of(arguments[0]));
    for (var operation : REQUIRED_OPERATIONS) {
      if (!source.contains("operationId: " + operation)) {
        throw new IllegalArgumentException("OpenAPI contract is missing operation " + operation);
      }
    }
    var output = Path.of(arguments[1]).resolve("dev/zonnedev/jman/example/contract/http/LibraryHttpContract.java");
    Files.createDirectories(output.getParent());
    Files.writeString(output, contractSource());
  }

  private static String contractSource() {
    return """
      package dev.zonnedev.jman.example.contract.http;

      import java.util.List;

      public final class LibraryHttpContract {
        public static final String USERS = "/users";
        public static final String USER = "/users/{userId}";
        public static final String USER_BOOKS = "/users/{userId}/books";
        public static final String BOOK_OWNER = "/books/{bookId}/owner";

        private LibraryHttpContract() {}

        public record CreateUserRequest(String name) {}

        public record RegisterBookRequest(String title) {}

        public record TransferBookRequest(String ownerId) {}

        public record UserResponse(String id, String name, List<String> bookIds) {
          public UserResponse {
            bookIds = List.copyOf(bookIds);
          }
        }

        public record BookResponse(String id, String title, String ownerId) {}
      }
      """;
  }
}
