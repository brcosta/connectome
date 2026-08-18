package dev.connectome;

public final class Greeter {
    public String greet(String name) {
        return decorate(name);
    }

    private String decorate(String name) {
        return "Hi " + name;
    }
}
