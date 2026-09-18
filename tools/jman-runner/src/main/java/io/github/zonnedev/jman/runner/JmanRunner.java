package io.github.zonnedev.jman.runner;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.Arrays;

/** Versioned, dependency-neutral entry point for isolated JUnit Platform runs. */
public final class JmanRunner {
    private static final String PROTOCOL_VERSION = "3";

    private JmanRunner() {}

    public static void main(String[] arguments) throws Exception {
        if (arguments.length < 3
                || !"--protocol".equals(arguments[0])
                || !PROTOCOL_VERSION.equals(arguments[1])
                || !"--".equals(arguments[2])) {
            System.err.println("jman-runner: expected --protocol 3 -- <JUnit Platform arguments>");
            System.exit(2);
        }

        String[] junitArguments = Arrays.copyOfRange(arguments, 3, arguments.length);
        Class<?> launcher = Class.forName("org.junit.platform.console.ConsoleLauncher");
        Method main = launcher.getMethod("main", String[].class);
        try {
            main.invoke(null, (Object) junitArguments);
        } catch (InvocationTargetException error) {
            Throwable cause = error.getCause();
            if (cause instanceof Exception) throw (Exception) cause;
            if (cause instanceof Error) throw (Error) cause;
            throw error;
        }
    }
}
