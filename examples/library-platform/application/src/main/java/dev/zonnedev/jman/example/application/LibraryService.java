package dev.zonnedev.jman.example.application;

import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.BookTitle;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import dev.zonnedev.jman.example.domain.UserName;
import java.util.List;

public final class LibraryService implements LibraryUseCases {
  private final UserRepository users;
  private final BookRepository books;

  public LibraryService(UserRepository users, BookRepository books) {
    this.users = users;
    this.books = books;
  }

  @Override
  public User createUser(String name) {
    var user = new User(UserId.create(), new UserName(name));
    users.save(user);
    return user;
  }

  @Override
  public User getUser(UserId id) {
    return users.findById(id)
      .orElseThrow(() -> {
        return new EntityNotFound("user " + id.value() + " was not found");
      });
  }

  @Override
  public List<User> listUsers() {
    return List.copyOf(users.findAll());
  }

  @Override
  public Book registerBook(UserId ownerId, String title) {
    var owner = getUser(ownerId);
    var book = new Book(BookId.create(), new BookTitle(title), ownerId);
    owner.addBook(book.id());
    users.save(owner);
    books.save(book);
    return book;
  }

  @Override
  public Book getBook(BookId id) {
    return books.findById(id)
      .orElseThrow(() -> {
        return new EntityNotFound("book " + id.value() + " was not found");
      });
  }

  @Override
  public List<Book> listBooks(UserId ownerId) {
    getUser(ownerId);
    return List.copyOf(books.findByOwner(ownerId));
  }

  @Override
  public Book transferBook(BookId bookId, UserId newOwnerId) {
    var book = getBook(bookId);
    var oldOwner = getUser(book.ownerId());
    var newOwner = getUser(newOwnerId);
    oldOwner.removeBook(bookId);
    newOwner.addBook(bookId);
    var transferred = book.transferTo(newOwnerId);
    users.save(oldOwner);
    users.save(newOwner);
    books.save(transferred);
    return transferred;
  }
}
