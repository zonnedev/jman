package dev.zonnedev.jman.example.grpc;

import dev.zonnedev.jman.example.application.BookRepository;
import dev.zonnedev.jman.example.application.UserRepository;
import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import jakarta.inject.Singleton;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

@Singleton
final class InMemoryLibraryStore implements UserRepository, BookRepository {
  private final Map<UserId, User> users = new LinkedHashMap<>();
  private final Map<BookId, Book> books = new LinkedHashMap<>();

  @Override
  public synchronized void save(User user) {
    users.put(user.id(), user);
  }

  @Override
  public synchronized void save(Book book) {
    books.put(book.id(), book);
  }

  @Override
  public synchronized Optional<User> findById(UserId id) {
    return Optional.ofNullable(users.get(id));
  }

  @Override
  public synchronized Optional<Book> findById(BookId id) {
    return Optional.ofNullable(books.get(id));
  }

  @Override
  public synchronized List<User> findAll() {
    return new ArrayList<>(users.values());
  }

  @Override
  public synchronized List<Book> findByOwner(UserId ownerId) {
    return books.values()
      .stream()
      .filter(book -> book.ownerId().equals(ownerId))
      .toList();
  }
}
