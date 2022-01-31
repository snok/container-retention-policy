FROM python:3.10.2-alpine

RUN apk add build-base

RUN pip install httpx dateparser pydantic

COPY main.py /main.py

ENTRYPOINT ["python", "/main.py"]
