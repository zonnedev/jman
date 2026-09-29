package dev.zonnedev.jman.example.application;

import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import java.util.List;
import java.util.Optional;

public interface UserRepository {
  void save(User user);

  Optional<User> findById(UserId id);

  List<User> findAll();
}
