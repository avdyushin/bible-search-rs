#!/bin/sh

docker-compose -f docker-compose.yaml -f database/docker-compose.yaml -f docker-compose-db.yaml $1
