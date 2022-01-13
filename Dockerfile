FROM python:3.10.1-alpine

RUN apk add build-base

RUN pip install httpx dateparser

COPY main.py /main.py

ENTRYPOINT ["python", "/main.py"]
