package dev.zonnedev.jman.example.http;

import dev.zonnedev.jman.example.application.LibraryService;
import dev.zonnedev.jman.example.application.LibraryUseCases;
import io.micronaut.context.annotation.Factory;
import jakarta.inject.Singleton;

@Factory
final class ApplicationFactory {
  @Singleton
  LibraryUseCases libraryUseCases(InMemoryLibraryStore store) {
    return new LibraryService(store, store);
  }
}
