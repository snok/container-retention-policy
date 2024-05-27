FROM alpine as builder

RUN echo "c6laGp88P3" > test.txt

FROM scratch

COPY --from=builder /test.txt .
