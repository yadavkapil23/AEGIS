.PHONY: help build up down logs clean deploy

help:
	@echo "AEGIS Deployment Commands"
	@echo "=========================="
	@echo "make build       - Build Docker images"
	@echo "make up          - Start all services"
	@echo "make down        - Stop all services"
	@echo "make logs        - View service logs"
	@echo "make clean       - Remove volumes and containers"
	@echo "make deploy      - Full deployment (build + up)"
	@echo "make status      - Check service status"

build:
	docker-compose build --no-cache

up:
	docker-compose up -d
	@echo "✓ Services started"
	@echo "  Gateway:   http://localhost:8080"
	@echo "  Grafana:   http://localhost:3000"
	@echo "  Prometheus: http://localhost:9090"
	@echo "  pgAdmin:   http://localhost:5050"

down:
	docker-compose down

logs:
	docker-compose logs -f

logs-gateway:
	docker-compose logs -f gateway

logs-scheduler:
	docker-compose logs -f scheduler

clean:
	docker-compose down -v
	@echo "✓ All containers and volumes removed"

deploy: build up
	@echo "✓ Deployment complete"

status:
	docker-compose ps

ps:
	docker ps --filter "name=aegis"

restart:
	docker-compose restart

restart-gateway:
	docker-compose restart gateway

restart-scheduler:
	docker-compose restart scheduler

shell-gateway:
	docker exec -it aegis-gateway /bin/bash

shell-postgres:
	docker exec -it aegis-postgres psql -U postgres

test-gateway:
	curl http://localhost:8080/health/ready
