package dev.zonnedev.jman.example.domain;

import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class BookTest {
  @Test
  void transferringABookCreatesANewOwnershipState() {
    var originalOwner = UserId.create();
    var newOwner = UserId.create();
    var book = new Book(BookId.create(), new BookTitle("Refactoring"), originalOwner);

    var transferred = book.transferTo(newOwner);

    Assertions.assertEquals(originalOwner, book.ownerId());
    Assertions.assertEquals(newOwner, transferred.ownerId());
    Assertions.assertEquals(book.id(), transferred.id());
  }

  @Test
  void valueObjectsRejectBlankBusinessValues() {
    Assertions.assertThrows(IllegalArgumentException.class, () -> new UserName("  "));
    Assertions.assertThrows(IllegalArgumentException.class, () -> new BookTitle(""));
  }
}
