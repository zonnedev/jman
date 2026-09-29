package dev.zonnedev.jman.example.domain;

public record Book(BookId id, BookTitle title, UserId ownerId) {
  public Book {
    if (id == null || title == null || ownerId == null) {
      throw new IllegalArgumentException("book id, title, and owner are required");
    }
  }

  public Book transferTo(UserId newOwnerId) {
    return new Book(id, title, newOwnerId);
  }
}
