plugins {
    application
}

repositories {
    maven {
        name = "publishingAcceptance"
        url = uri(providers.gradleProperty("publishingRepository").get())
    }
}

dependencies {
    implementation("io.github.zonnedev.jman.fixture:greeting-library:1.0.0")
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

application {
    mainClass = "io.github.zonnedev.jman.fixture.consumer.Application"
}
