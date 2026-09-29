package dev.zonnedev.jman.example.contract;

import dev.zonnedev.jman.example.contract.grpc.LibraryGrpcContract;
import dev.zonnedev.jman.example.contract.http.LibraryHttpContract;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class GeneratedContractsTest {
  @Test
  void exposesHttpAndGrpcOperationsFromGeneratedSources() {
    Assertions.assertEquals("/users", LibraryHttpContract.USERS);
    Assertions.assertEquals(6, LibraryGrpcContract.serviceDescriptor().getMethods().size());
  }
}
