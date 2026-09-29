package dev.zonnedev.jman.example.domain;

import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class UserTest {
  @Test
  void tracksOwnedBooksWithoutExposingMutableState() {
    var user = new User(UserId.create(), new UserName("Ada"));
    var bookId = BookId.create();

    user.addBook(bookId);

    Assertions.assertEquals(1, user.bookIds().size());
    Assertions.assertThrows(UnsupportedOperationException.class, () -> user.bookIds().clear());
  }

  @Test
  void rejectsOwningTheSameBookTwice() {
    var user = new User(UserId.create(), new UserName("Ada"));
    var bookId = BookId.create();
    user.addBook(bookId);

    Assertions.assertThrows(DomainConflict.class, () -> user.addBook(bookId));
  }
}
