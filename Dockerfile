FROM rust:latest

WORKDIR /usr/src/app

COPY . /usr/src/app

RUN apt update && apt upgrade -y

RUN apt install -y protobuf-compiler libprotobuf-dev

RUN cargo install --path .