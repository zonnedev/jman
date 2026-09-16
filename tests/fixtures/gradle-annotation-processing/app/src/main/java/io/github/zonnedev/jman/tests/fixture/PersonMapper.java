package io.github.zonnedev.jman.tests.fixture;

import org.mapstruct.Mapper;

@Mapper
public interface PersonMapper {
  PersonDto toDto(Person person);
}
