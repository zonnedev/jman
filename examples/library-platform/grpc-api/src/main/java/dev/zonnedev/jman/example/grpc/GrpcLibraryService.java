package dev.zonnedev.jman.example.grpc;

import dev.zonnedev.jman.example.application.LibraryUseCases;
import dev.zonnedev.jman.example.contract.grpc.LibraryGrpcContract;
import dev.zonnedev.jman.example.contract.grpc.LibraryProto;
import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import io.grpc.BindableService;
import io.grpc.ServerServiceDefinition;
import io.grpc.stub.ServerCalls;
import jakarta.inject.Singleton;

@Singleton
public final class GrpcLibraryService implements BindableService {
  private final LibraryUseCases library;

  public GrpcLibraryService(LibraryUseCases library) {
    this.library = library;
  }

  @Override
  public ServerServiceDefinition bindService() {
    return ServerServiceDefinition.builder(LibraryGrpcContract.serviceDescriptor())
      .addMethod(
      LibraryGrpcContract.CREATE_USER,
      ServerCalls.asyncUnaryCall((request, observer) -> {
        observer.onNext(user(library.createUser(request.getName())));
        observer.onCompleted();
      })
    )
      .addMethod(
      LibraryGrpcContract.GET_USER,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        observer.onNext(user(library.getUser(UserId.parse(request.getUserId()))));
        observer.onCompleted();
      }
    )
    )
      .addMethod(
      LibraryGrpcContract.LIST_USERS,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        var response = LibraryProto.ListUsersResponse.newBuilder();
        library.listUsers().stream().map(GrpcLibraryService::user).forEach(response::addUsers);
        observer.onNext(response.build());
        observer.onCompleted();
      }
    )
    )
      .addMethod(
      LibraryGrpcContract.REGISTER_BOOK,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        var created = library.registerBook(UserId.parse(request.getOwnerId()), request.getTitle());
        observer.onNext(book(created));
        observer.onCompleted();
      }
    )
    )
      .addMethod(
      LibraryGrpcContract.LIST_BOOKS,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        var response = LibraryProto.ListBooksResponse.newBuilder();
        library.listBooks(UserId.parse(request.getOwnerId()))
          .stream()
          .map(GrpcLibraryService::book)
          .forEach(response::addBooks);
        observer.onNext(response.build());
        observer.onCompleted();
      }
    )
    )
      .addMethod(
      LibraryGrpcContract.TRANSFER_BOOK,
      ServerCalls.asyncUnaryCall(
      (request, observer) -> {
        var transferred = library.transferBook(BookId.parse(request.getBookId()), UserId.parse(request.getNewOwnerId()));
        observer.onNext(book(transferred));
        observer.onCompleted();
      }
    )
    )
      .build();
  }

  private static LibraryProto.UserResponse user(User user) {
    var response = LibraryProto.UserResponse.newBuilder()
      .setId(user.id().value().toString())
      .setName(user.name().value());
    user.bookIds()
      .stream()
      .map(id -> id.value().toString())
      .forEach(response::addBookIds);
    return response.build();
  }

  private static LibraryProto.BookResponse book(Book book) {
    return LibraryProto.BookResponse.newBuilder()
      .setId(book.id().value().toString())
      .setTitle(book.title().value())
      .setOwnerId(book.ownerId().value().toString())
      .build();
  }
}
