package dev.zonnedev.jman.example.grpc;

import dev.zonnedev.jman.example.application.LibraryService;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class GrpcLibraryServiceTest {
  @Test
  void bindsEveryOperationFromTheProtobufContract() {
    var store = new InMemoryLibraryStore();
    var service = new GrpcLibraryService(new LibraryService(store, store));

    Assertions.assertEquals(6, service.bindService().getMethods().size());
  }
}
