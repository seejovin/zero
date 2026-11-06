plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.localwallet.mainnet"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = "27.2.12479018"
    defaultConfig {
        applicationId = "app.localwallet.mainnet"
        minSdk = 30
        targetSdk = 35
        versionCode = 16
        versionName = "0.13.0-mainnet"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    buildTypes {
        getByName("release") {
            isDebuggable = false
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
        getByName("debug") { applicationIdSuffix = ".debug" }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].java.srcDir("src/main/generated")
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/privateEngineAssets"))
    packaging { jniLibs { useLegacyPackaging = false } }
    lint { abortOnError = true; checkReleaseBuilds = true }
    testOptions {
        unitTests.all {
            val library = if (System.getProperty("os.name").startsWith("Mac")) "libwallet_core.dylib" else "libwallet_core.so"
            it.systemProperty("uniffi.component.wallet_core.libraryOverride", rootDir.parentFile.resolve("target/release/$library").absolutePath)
        }
    }
}

// The private engine is bundled into the APK. It never downloads executable
// JavaScript or proving parameters at runtime. A protocol manifest does not
// activate the separate, source-reviewed native release gate.
val privateEngineRoot = rootProject.projectDir.parentFile.resolve("privacy-engine")
val privateEngineAssets = layout.buildDirectory.dir("generated/privateEngineAssets/private-engine")
val privateProtocolManifest = providers.gradleProperty("privateProtocolManifest")
val installPrivateEngineDependencies by tasks.registering(Exec::class) {
    workingDir(privateEngineRoot)
    commandLine("npm", "ci", "--ignore-scripts", "--no-fund", "--no-audit")
    inputs.files(privateEngineRoot.resolve("package.json"), privateEngineRoot.resolve("package-lock.json"))
    inputs.dir(privateEngineRoot.resolve("vendor"))
    outputs.dir(privateEngineRoot.resolve("node_modules"))
}
val bundlePrivateEngine by tasks.registering(Exec::class) {
    dependsOn(installPrivateEngineDependencies)
    workingDir(privateEngineRoot)
    listOf("src", "public", "config", "scripts").forEach {
        inputs.dir(privateEngineRoot.resolve(it))
    }
    inputs.files(privateEngineRoot.resolve("package.json"), privateEngineRoot.resolve("package-lock.json"))
    if (privateProtocolManifest.isPresent) {
        val file = file(privateProtocolManifest.get())
        inputs.file(file)
        inputs.dir(file.parentFile.resolve("artifacts"))
    }
    outputs.dir(privateEngineAssets)
    doFirst {
        val args = mutableListOf("node", "scripts/build-browser.mjs", privateEngineAssets.get().asFile.absolutePath)
        if (privateProtocolManifest.isPresent) args.add(file(privateProtocolManifest.get()).absolutePath)
        commandLine(args)
    }
}
tasks.named("preBuild").configure { dependsOn(bundlePrivateEngine) }

dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("com.google.zxing:core:3.5.3")
    implementation("androidx.annotation:annotation:1.9.1")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20250107")
    testRuntimeOnly("net.java.dev.jna:jna:5.17.0@jar")
}
