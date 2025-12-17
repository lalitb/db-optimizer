#!/bin/bash
set -e

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}=== Vaanyze Fleet Simulation Setup ===${NC}"

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "Error: Docker is not running. Please start Docker Desktop."
    exit 1
fi

# Create a network if it doesn't exist
docker network create dbopt-network 2>/dev/null || true

echo -e "${YELLOW}Step 1: Spinning up 3 PostgreSQL instances (Primary, Replica, Analytics)...${NC}"

# Create a temporary compose file for the fleet
cat <<EOF > docker-compose-fleet.yml
version: '3.8'

services:
  # Instance 1: Primary Transactional DB
  primary-db:
    image: postgres:16-alpine
    container_name: vaanyze-primary
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: password
      POSTGRES_DB: primary_db
    ports:
      - "5432:5432"
    volumes:
      - ./scripts/test-data.sql:/docker-entrypoint-initdb.d/01-test-data.sql
    networks:
      - dbopt-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  # Instance 2: Read Replica (Simulated)
  replica-db:
    image: postgres:16-alpine
    container_name: vaanyze-replica
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: password
      POSTGRES_DB: replica_db
    ports:
      - "5433:5432"
    volumes:
      - ./scripts/test-data.sql:/docker-entrypoint-initdb.d/01-test-data.sql
    networks:
      - dbopt-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

  # Instance 3: Analytics DB (Different workload)
  analytics-db:
    image: postgres:16-alpine
    container_name: vaanyze-analytics
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: password
      POSTGRES_DB: analytics_db
    ports:
      - "5434:5432"
    volumes:
      - ./scripts/test-data.sql:/docker-entrypoint-initdb.d/01-test-data.sql
    networks:
      - dbopt-network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 5

networks:
  dbopt-network:
    external: true
EOF

# Start the fleet
docker-compose -f docker-compose-fleet.yml up -d

echo -e "${YELLOW}Step 2: Waiting for databases to be ready...${NC}"
sleep 10 # Give them a moment to start initializing
# Wait for healthchecks
echo "Waiting for Primary..."
until docker exec vaanyze-primary pg_isready -U postgres > /dev/null 2>&1; do sleep 2; done
echo "Waiting for Replica..."
until docker exec vaanyze-replica pg_isready -U postgres > /dev/null 2>&1; do sleep 2; done
echo "Waiting for Analytics..."
until docker exec vaanyze-analytics pg_isready -U postgres > /dev/null 2>&1; do sleep 2; done

echo -e "${GREEN}All databases are up and running!${NC}"

echo -e "${YELLOW}Step 3: Registering instances with Vaanyze Fleet Manager...${NC}"

# Ensure the server is running (User needs to run 'cargo run' in another terminal)
# We'll try to register assuming it's on localhost:9090
API_URL="http://localhost:9090/api/v1/fleet"

# Function to register instance
register_db() {
    NAME=$1
    CONN=$2
    ENV=$3
    TAG=$4
    
    curl -s -X POST $API_URL \
      -H "Content-Type: application/json" \
      -d "{
        \"name\": \"$NAME\",
        \"connection_string\": \"$CONN\",
        \"environment\": \"$ENV\",
        \"tags\": [\"$TAG\"]
      }" > /dev/null
    echo "Registered $NAME"
}

echo "Attempting to register instances (Make sure 'cargo run' is running!)..."

# Note: We use 'localhost' for connection strings because the Rust app is running on the host,
# and the ports are mapped to localhost (5432, 5433, 5434).
register_db "Primary-DB" "postgresql://postgres:password@localhost:5432/primary_db" "Production" "region:us-east-1" || echo "Failed to register. Is server running?"
register_db "Replica-DB" "postgresql://postgres:password@localhost:5433/replica_db" "Production" "region:us-east-1" || echo "Failed to register."
register_db "Analytics-DB" "postgresql://postgres:password@localhost:5434/analytics_db" "Staging" "team:data-science" || echo "Failed to register."

echo -e "${YELLOW}Step 4: Generating Traffic (Background)...${NC}"

# Create a traffic generator script
cat <<EOF > generate_traffic.sh
#!/bin/bash
while true; do
    # Primary: Transactional load (Inserts/Updates)
    docker exec vaanyze-primary psql -U postgres -d primary_db -c "INSERT INTO orders (customer_id, total_amount) VALUES (1, random()*100);" >/dev/null 2>&1
    
    # Replica: Read load (Slow queries)
    docker exec vaanyze-replica psql -U postgres -d replica_db -f /docker-entrypoint-initdb.d/01-test-data.sql >/dev/null 2>&1 &
    
    # Analytics: Heavy aggregation
    docker exec vaanyze-analytics psql -U postgres -d analytics_db -c "SELECT count(*) FROM order_items;" >/dev/null 2>&1
    
    sleep 2
done
EOF
chmod +x generate_traffic.sh

echo -e "${GREEN}Setup Complete!${NC}"
echo "1. Run 'cargo run' in a separate terminal to start the Vaanyze Server."
echo "2. Run './generate_traffic.sh' to simulate load."
echo "3. Open http://localhost:9090/dashboard.html to see the fleet in action."
