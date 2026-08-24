package com.patterns.structural.facade;

public class VideoConversionFacade {
    private AudioMixer audioMixer = new AudioMixer();
    private BitrateReader bitrateReader = new BitrateReader();

    public String convertVideo(String fileName, String format) {
        System.out.println("VideoConversionFacade: conversion started.");
        bitrateReader.read(fileName);
        audioMixer.fix();
        System.out.println("VideoConversionFacade: conversion completed.");
        return "ConvertedVideo." + format;
    }
}
