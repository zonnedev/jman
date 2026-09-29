package dev.zonnedev.jman.example.http;

import dev.zonnedev.jman.example.application.LibraryUseCases;
import dev.zonnedev.jman.example.contract.http.LibraryHttpContract;
import dev.zonnedev.jman.example.domain.Book;
import dev.zonnedev.jman.example.domain.BookId;
import dev.zonnedev.jman.example.domain.User;
import dev.zonnedev.jman.example.domain.UserId;
import io.micronaut.http.HttpStatus;
import io.micronaut.http.annotation.Body;
import io.micronaut.http.annotation.Controller;
import io.micronaut.http.annotation.Get;
import io.micronaut.http.annotation.PathVariable;
import io.micronaut.http.annotation.Post;
import io.micronaut.http.annotation.Put;
import io.micronaut.http.annotation.Status;
import java.util.List;

@Controller
public final class LibraryController {
  private final LibraryUseCases library;

  public LibraryController(LibraryUseCases library) {
    this.library = library;
  }

  @Post(LibraryHttpContract.USERS)
  @Status(HttpStatus.CREATED)
  public LibraryHttpContract.UserResponse createUser(
    @Body LibraryHttpContract.CreateUserRequest request
  ) {
    return user(library.createUser(request.name()));
  }

  @Get(LibraryHttpContract.USERS)
  public List<LibraryHttpContract.UserResponse> listUsers() {
    return library.listUsers().stream().map(LibraryController::user).toList();
  }

  @Get(LibraryHttpContract.USER)
  public LibraryHttpContract.UserResponse getUser(@PathVariable String userId) {
    return user(library.getUser(UserId.parse(userId)));
  }

  @Post(LibraryHttpContract.USER_BOOKS)
  @Status(HttpStatus.CREATED)
  public LibraryHttpContract.BookResponse registerBook(
    @PathVariable String userId,
    @Body LibraryHttpContract.RegisterBookRequest request
  ) {
    return book(library.registerBook(UserId.parse(userId), request.title()));
  }

  @Get(LibraryHttpContract.USER_BOOKS)
  public List<LibraryHttpContract.BookResponse> listBooks(@PathVariable String userId) {
    return library.listBooks(UserId.parse(userId))
      .stream()
      .map(LibraryController::book)
      .toList();
  }

  @Put(LibraryHttpContract.BOOK_OWNER)
  public LibraryHttpContract.BookResponse transferBook(
    @PathVariable String bookId,
    @Body LibraryHttpContract.TransferBookRequest request
  ) {
    return book(library.transferBook(BookId.parse(bookId), UserId.parse(request.ownerId())));
  }

  private static LibraryHttpContract.UserResponse user(User user) {
    return new LibraryHttpContract.UserResponse(user.id().value().toString(), user.name().value(), user.bookIds()
      .stream()
      .map(id -> id.value().toString())
      .toList());
  }

  private static LibraryHttpContract.BookResponse book(Book book) {
    return new LibraryHttpContract.BookResponse(book.id().value().toString(), book.title().value(), book.ownerId().value().toString());
  }
}
