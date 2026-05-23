# AEGIS Deployment Guide

## Quick Start

### Prerequisites
- Docker & Docker Compose installed
- 8GB RAM minimum
- 20GB disk space

### 1. **Build Images**
```bash
cd C:\Users\ky805\Downloads\AI-Project
docker-compose build --no-cache
```

### 2. **Start All Services**
```bash
docker-compose up -d
```

### 3. **Verify Deployment**
```bash
# Gateway health
curl http://localhost:8080/health/ready

# Service URLs:
# Gateway:     http://localhost:8080
# Grafana:     http://localhost:3000 (admin/admin)
# Prometheus:  http://localhost:9090
# pgAdmin:     http://localhost:5050
```

---

## Service Stack

| Service | Port | Purpose |
|---------|------|---------|
| Gateway | 8080 | HTTP API Server |
| Scheduler | 50051 | gRPC Service |
| PostgreSQL | 5432 | Database |
| Redis | 6379 | Cache |
| Prometheus | 9090 | Metrics |
| Grafana | 3000 | Dashboards |
| pgAdmin | 5050 | DB Management |

---

## Common Commands

```bash
# View logs
docker-compose logs -f

# View specific service
docker-compose logs -f gateway

# Stop services
docker-compose down

# Status
docker-compose ps

# Clean everything
docker-compose down -v

# Restart service
docker-compose restart gateway

# Database access
docker exec -it aegis-postgres psql -U postgres

# Backup database
docker exec aegis-postgres pg_dump -U postgres aegis_gateway > backup.sql
```

---

## Environment Setup

Copy `.env.example` to `.env` and customize:

```bash
cp .env.example .env
# Edit .env with your values
docker-compose up -d
```

---

## Production Checklist

- [ ] Change default passwords in `.env`
- [ ] Use strong PostgreSQL password
- [ ] Set `RUST_LOG=warn`
- [ ] Enable TLS/SSL
- [ ] Backup PostgreSQL regularly
- [ ] Monitor logs
- [ ] Use firewall rules

---

## Troubleshooting

**Gateway won't start:**
```bash
docker-compose logs gateway
# Check: PostgreSQL ready? Port 8080 free? DATABASE_URL correct?
```

**PostgreSQL connection error:**
```bash
docker exec aegis-postgres psql -U postgres -c "SELECT 1"
```

**Out of disk space:**
```bash
docker volume prune
docker system df
```

**Reset everything:**
```bash
docker-compose down -v
docker-compose build --no-cache
docker-compose up -d
```

---

## More Information

See DEPLOYMENT.md for:
- Monitoring setup
- Performance tuning
- Scaling strategies
- CI/CD integration
- Backup & restore
