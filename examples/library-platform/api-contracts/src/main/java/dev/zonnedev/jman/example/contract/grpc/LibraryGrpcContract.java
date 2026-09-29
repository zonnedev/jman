package dev.zonnedev.jman.example.contract.grpc;

import com.google.protobuf.Message;
import io.grpc.MethodDescriptor;
import io.grpc.ServiceDescriptor;
import io.grpc.protobuf.ProtoUtils;

public final class LibraryGrpcContract {
  public static final String SERVICE = "jman.example.library.v1.LibraryService";

  public static final MethodDescriptor<LibraryProto.CreateUserRequest, LibraryProto.UserResponse> CREATE_USER = unary("CreateUser", LibraryProto.CreateUserRequest.getDefaultInstance(), LibraryProto.UserResponse.getDefaultInstance());
  public static final MethodDescriptor<LibraryProto.GetUserRequest, LibraryProto.UserResponse> GET_USER = unary("GetUser", LibraryProto.GetUserRequest.getDefaultInstance(), LibraryProto.UserResponse.getDefaultInstance());
  public static final MethodDescriptor<LibraryProto.ListUsersRequest, LibraryProto.ListUsersResponse> LIST_USERS = unary("ListUsers", LibraryProto.ListUsersRequest.getDefaultInstance(), LibraryProto.ListUsersResponse.getDefaultInstance());
  public static final MethodDescriptor<LibraryProto.RegisterBookRequest, LibraryProto.BookResponse> REGISTER_BOOK = unary("RegisterBook", LibraryProto.RegisterBookRequest.getDefaultInstance(), LibraryProto.BookResponse.getDefaultInstance());
  public static final MethodDescriptor<LibraryProto.ListBooksRequest, LibraryProto.ListBooksResponse> LIST_BOOKS = unary("ListBooks", LibraryProto.ListBooksRequest.getDefaultInstance(), LibraryProto.ListBooksResponse.getDefaultInstance());
  public static final MethodDescriptor<LibraryProto.TransferBookRequest, LibraryProto.BookResponse> TRANSFER_BOOK = unary("TransferBook", LibraryProto.TransferBookRequest.getDefaultInstance(), LibraryProto.BookResponse.getDefaultInstance());

  private LibraryGrpcContract() {
  }

  public static ServiceDescriptor serviceDescriptor() {
    return ServiceDescriptor.newBuilder(SERVICE)
      .addMethod(CREATE_USER)
      .addMethod(GET_USER)
      .addMethod(LIST_USERS)
      .addMethod(REGISTER_BOOK)
      .addMethod(LIST_BOOKS)
      .addMethod(TRANSFER_BOOK)
      .build();
  }

  private static <RequestT, ResponseT> MethodDescriptor<RequestT, ResponseT> unary(String method, Message request, Message response) {
    @SuppressWarnings("unchecked")
    var requestMarshaller = (MethodDescriptor.Marshaller<RequestT>)ProtoUtils.marshaller(request);
    @SuppressWarnings("unchecked")
    var responseMarshaller = (MethodDescriptor.Marshaller<ResponseT>)ProtoUtils.marshaller(response);
    return MethodDescriptor.<RequestT, ResponseT>newBuilder()
      .setType(MethodDescriptor.MethodType.UNARY)
      .setFullMethodName(MethodDescriptor.generateFullMethodName(SERVICE, method))
      .setRequestMarshaller(requestMarshaller)
      .setResponseMarshaller(responseMarshaller)
      .build();
  }
}
