package dev.zonnedev.jman.example.application;

import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class LibraryServiceTest {
  private InMemoryUsers users;
  private InMemoryBooks books;
  private LibraryService service;

  @BeforeEach
  void setUp() {
    users = new InMemoryUsers();
    books = new InMemoryBooks();
    service = new LibraryService(users, books);
  }

  @Test
  void registersABookForAnExistingOwner() {
    var owner = service.createUser("Ada");

    var book = service.registerBook(owner.id(), "Domain-Driven Design");

    Assertions.assertEquals(owner.id(), book.ownerId());
    Assertions.assertEquals(List.of(book), service.listBooks(owner.id()));
  }

  @Test
  void transfersOwnershipAtomicallyAtTheUseCaseBoundary() {
    var ada = service.createUser("Ada");
    var grace = service.createUser("Grace");
    var book = service.registerBook(ada.id(), "Designing Data-Intensive Applications");

    var transferred = service.transferBook(book.id(), grace.id());

    Assertions.assertEquals(grace.id(), transferred.ownerId());
    Assertions.assertTrue(service.listBooks(ada.id()).isEmpty());
    Assertions.assertEquals(List.of(transferred), service.listBooks(grace.id()));
  }

  @Test
  void refusesToRegisterABookForAMissingOwner() {
    Assertions.assertThrows(EntityNotFound.class, () -> {
      service.registerBook(UserId.create(), "Clean Architecture");
    });
  }

  private static final class InMemoryUsers implements UserRepository {
    private final Map<UserId, User> values = new LinkedHashMap<>();

    @Override
    public void save(User user) {
      values.put(user.id(), user);
    }

    @Override
    public Optional<User> findById(UserId id) {
      return Optional.ofNullable(values.get(id));
    }

    @Override
    public List<User> findAll() {
      return new ArrayList<>(values.values());
    }
  }

  private static final class InMemoryBooks implements BookRepository {
    private final Map<BookId, Book> values = new LinkedHashMap<>();

    @Override
    public void save(Book book) {
      values.put(book.id(), book);
    }

    @Override
    public Optional<Book> findById(BookId id) {
      return Optional.ofNullable(values.get(id));
    }

    @Override
    public List<Book> findByOwner(UserId ownerId) {
      return values.values()
        .stream()
        .filter(book -> book.ownerId().equals(ownerId))
        .toList();
    }
  }
}
