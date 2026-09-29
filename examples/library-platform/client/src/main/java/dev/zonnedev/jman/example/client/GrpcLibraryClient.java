package dev.zonnedev.jman.example.client;

import dev.zonnedev.jman.example.contract.grpc.LibraryGrpcContract;
import dev.zonnedev.jman.example.contract.grpc.LibraryProto;
import io.grpc.CallOptions;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;
import io.grpc.MethodDescriptor;
import io.grpc.stub.ClientCalls;
import java.util.List;
import java.util.concurrent.TimeUnit;

final class GrpcLibraryClient implements LibraryClient {
  private final ManagedChannel channel;

  GrpcLibraryClient(String host, int port) {
    channel = ManagedChannelBuilder.forAddress(host, port).usePlaintext().build();
  }

  @Override
  public ClientUser createUser(String name) {
    var request = LibraryProto.CreateUserRequest.newBuilder().setName(name).build();
    return user(call(LibraryGrpcContract.CREATE_USER, request));
  }

  @Override
  public List<ClientUser> listUsers() {
    var request = LibraryProto.ListUsersRequest.getDefaultInstance();
    return call(LibraryGrpcContract.LIST_USERS, request)
      .getUsersList()
      .stream()
      .map(GrpcLibraryClient::user)
      .toList();
  }

  @Override
  public ClientBook registerBook(String ownerId, String title) {
    var request = LibraryProto.RegisterBookRequest.newBuilder().setOwnerId(ownerId).setTitle(title).build();
    return book(call(LibraryGrpcContract.REGISTER_BOOK, request));
  }

  @Override
  public List<ClientBook> listBooks(String ownerId) {
    var request = LibraryProto.ListBooksRequest.newBuilder().setOwnerId(ownerId).build();
    return call(LibraryGrpcContract.LIST_BOOKS, request)
      .getBooksList()
      .stream()
      .map(GrpcLibraryClient::book)
      .toList();
  }

  @Override
  public ClientBook transferBook(String bookId, String newOwnerId) {
    var request = LibraryProto.TransferBookRequest.newBuilder().setBookId(bookId).setNewOwnerId(newOwnerId).build();
    return book(call(LibraryGrpcContract.TRANSFER_BOOK, request));
  }

  @Override
  public void close() {
    channel.shutdown();
    try {
      if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
        channel.shutdownNow();
      }
    } catch (InterruptedException exception) {
      Thread.currentThread().interrupt();
      channel.shutdownNow();
    }
  }

  private <RequestT, ResponseT> ResponseT call(MethodDescriptor<RequestT, ResponseT> method, RequestT request) {
    return ClientCalls.blockingUnaryCall(channel, method, CallOptions.DEFAULT, request);
  }

  private static ClientUser user(LibraryProto.UserResponse response) {
    return new ClientUser(response.getId(), response.getName(), response.getBookIdsList());
  }

  private static ClientBook book(LibraryProto.BookResponse response) {
    return new ClientBook(response.getId(), response.getTitle(), response.getOwnerId());
  }
}
