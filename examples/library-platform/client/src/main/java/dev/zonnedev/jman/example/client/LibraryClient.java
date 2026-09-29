package dev.zonnedev.jman.example.client;

import java.util.List;

public interface LibraryClient extends AutoCloseable {
  ClientUser createUser(String name);

  List<ClientUser> listUsers();

  ClientBook registerBook(String ownerId, String title);

  List<ClientBook> listBooks(String ownerId);

  ClientBook transferBook(String bookId, String newOwnerId);

  @Override
  void close();

  static LibraryClientBuilder builder() {
    return new LibraryClientBuilder();
  }
}
