for ((i=1; i<=5; i++))
do
  imageName="ghcr.io/snok/container-retention-policy:test-${i}"
  randomString=$(LC_ALL=C tr -dc A-Za-z0-9 </dev/urandom | head -c 10)
  echo "\
FROM alpine as builder

RUN echo \"$randomString\" > test.txt

FROM scratch

COPY --from=builder /test.txt ." > Dockerfile
  docker buildx build -f Dockerfile -t $imageName --push .

  for ((j=1; j<=3; j++))
  do
    newName="ghcr.io/snok/container-retention-policy:test-${i}-${j}"
    docker tag $imageName $newName
    docker push $newName
  done
done
