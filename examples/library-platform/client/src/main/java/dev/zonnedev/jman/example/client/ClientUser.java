package dev.zonnedev.jman.example.client;

import java.util.List;

public record ClientUser(String id, String name, List<String> bookIds) {
  public ClientUser {
    bookIds = List.copyOf(bookIds);
  }
}
