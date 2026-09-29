package dev.zonnedev.jman.example.application;

import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import java.util.List;

public interface LibraryUseCases {
  User createUser(String name);

  User getUser(UserId id);

  List<User> listUsers();

  Book registerBook(UserId ownerId, String title);

  Book getBook(BookId id);

  List<Book> listBooks(UserId ownerId);

  Book transferBook(BookId bookId, UserId newOwnerId);
}
