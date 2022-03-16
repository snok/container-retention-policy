FROM python:3.10.2-alpine

RUN apk add build-base

RUN pip install \
    # Added after new regex release broke \
    # dateparser. See https://stackoverflow.com/questions/71496687/dateparser-throws-regex-regex-core-error/71501074#71501074 \
    # and https://github.com/snok/container-retention-policy/issues/26
    regex==2022.3.2 \
    httpx \
    dateparser \
    pydantic

COPY main.py /main.py

ENTRYPOINT ["python", "/main.py"]
