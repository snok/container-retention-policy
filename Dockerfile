FROM alpine as builder

RUN echo "lugVvR2ORv" > test.txt

FROM scratch

COPY --from=builder /test.txt .
