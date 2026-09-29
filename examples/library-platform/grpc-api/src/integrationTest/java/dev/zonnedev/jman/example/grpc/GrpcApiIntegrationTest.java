package dev.zonnedev.jman.example.grpc;

import dev.zonnedev.jman.example.application.LibraryService;
import dev.zonnedev.jman.example.contract.grpc.LibraryGrpcContract;
import dev.zonnedev.jman.example.contract.grpc.LibraryProto;
import io.grpc.CallOptions;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;
import io.grpc.MethodDescriptor;
import io.grpc.ServerBuilder;
import io.grpc.stub.ClientCalls;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Assertions;
import org.junit.jupiter.api.Test;

class GrpcApiIntegrationTest {
  @Test
  void servesTheGeneratedProtobufContractOverARealSocket() throws Exception {
    var store = new InMemoryLibraryStore();
    var service = new GrpcLibraryService(new LibraryService(store, store));
    var server = ServerBuilder.forPort(0).addService(service).build().start();
    var channel = ManagedChannelBuilder.forAddress("127.0.0.1", server.getPort())
      .usePlaintext()
      .build();
    try {
      var ada = call(channel, LibraryGrpcContract.CREATE_USER, LibraryProto.CreateUserRequest.newBuilder().setName("Ada").build());
      var grace = call(channel, LibraryGrpcContract.CREATE_USER, LibraryProto.CreateUserRequest.newBuilder().setName("Grace").build());
      var found = call(channel, LibraryGrpcContract.GET_USER, LibraryProto.GetUserRequest.newBuilder()
        .setUserId(ada.getId())
        .build());
      var users = call(channel, LibraryGrpcContract.LIST_USERS, LibraryProto.ListUsersRequest.getDefaultInstance());
      var book = call(
        channel,
        LibraryGrpcContract.REGISTER_BOOK,
        LibraryProto.RegisterBookRequest.newBuilder()
        .setOwnerId(ada.getId())
        .setTitle("Domain-Driven Design")
        .build()
      );
      var transferred = call(
        channel,
        LibraryGrpcContract.TRANSFER_BOOK,
        LibraryProto.TransferBookRequest.newBuilder()
        .setBookId(book.getId())
        .setNewOwnerId(grace.getId())
        .build()
      );
      var books = call(channel, LibraryGrpcContract.LIST_BOOKS, LibraryProto.ListBooksRequest.newBuilder()
        .setOwnerId(grace.getId())
        .build());

      Assertions.assertEquals("Ada", found.getName());
      Assertions.assertEquals(2, users.getUsersCount());
      Assertions.assertEquals(grace.getId(), transferred.getOwnerId());
      Assertions.assertEquals(book.getId(), books.getBooks(0).getId());
    } finally {
      channel.shutdownNow().awaitTermination(5, TimeUnit.SECONDS);
      server.shutdownNow().awaitTermination(5, TimeUnit.SECONDS);
    }
  }

  private static <RequestT, ResponseT> ResponseT call(ManagedChannel channel, MethodDescriptor<RequestT, ResponseT> method, RequestT request) {
    return ClientCalls.blockingUnaryCall(channel, method, CallOptions.DEFAULT, request);
  }
}
