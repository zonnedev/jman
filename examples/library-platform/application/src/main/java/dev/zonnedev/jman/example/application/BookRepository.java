package dev.zonnedev.jman.example.application;

import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.UserId;
import java.util.List;
import java.util.Optional;

public interface BookRepository {
  void save(Book book);

  Optional<Book> findById(BookId id);

  List<Book> findByOwner(UserId ownerId);
}
