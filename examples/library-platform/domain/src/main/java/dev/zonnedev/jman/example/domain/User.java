package dev.zonnedev.jman.example.domain;

import java.util.LinkedHashSet;
import java.util.Set;

public final class User {
  private final UserId id;
  private final UserName name;
  private final Set<BookId> bookIds;

  public User(UserId id, UserName name) {
    this(id, name, Set.of());
  }

  public User(UserId id, UserName name, Set<BookId> bookIds) {
    if (id == null || name == null || bookIds == null) {
      throw new IllegalArgumentException("user id, name, and books are required");
    }
    this.id = id;
    this.name = name;
    this.bookIds = new LinkedHashSet<>(bookIds);
  }

  public UserId id() {
    return id;
  }

  public UserName name() {
    return name;
  }

  public Set<BookId> bookIds() {
    return Set.copyOf(bookIds);
  }

  public void addBook(BookId bookId) {
    if (!bookIds.add(bookId)) {
      throw new DomainConflict("book is already owned by this user");
    }
  }

  public void removeBook(BookId bookId) {
    if (!bookIds.remove(bookId)) {
      throw new DomainConflict("book is not owned by this user");
    }
  }
}
