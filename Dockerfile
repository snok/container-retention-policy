FROM alpine as builder

RUN echo "OGD5JGXed4" > test.txt

FROM scratch

COPY --from=builder /test.txt .
