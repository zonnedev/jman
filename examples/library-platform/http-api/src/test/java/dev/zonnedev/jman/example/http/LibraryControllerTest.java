package dev.zonnedev.jman.example.http;

import dev.zonnedev.jman.example.application.LibraryService;
import dev.zonnedev.jman.example.contract.http.LibraryHttpContract;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class LibraryControllerTest {
  @Test
  void mapsUseCaseResultsToTheGeneratedHttpContract() {
    var store = new InMemoryLibraryStore();
    var controller = new LibraryController(new LibraryService(store, store));

    var created = controller.createUser(new LibraryHttpContract.CreateUserRequest("Ada"));
    var newOwner = controller.createUser(new LibraryHttpContract.CreateUserRequest("Grace"));
    var book = controller.registerBook(created.id(), new LibraryHttpContract.RegisterBookRequest("Domain-Driven Design"));
    var transferred = controller.transferBook(book.id(), new LibraryHttpContract.TransferBookRequest(newOwner.id()));

    Assertions.assertEquals("Ada", created.name());
    Assertions.assertEquals(created.id(), book.ownerId());
    Assertions.assertEquals(created, controller.getUser(created.id()));
    Assertions.assertEquals(2, controller.listUsers().size());
    Assertions.assertTrue(controller.listBooks(created.id()).isEmpty());
    Assertions.assertEquals(newOwner.id(), transferred.ownerId());
    Assertions.assertEquals(1, controller.listBooks(newOwner.id()).size());
  }
}
