# Pakyas User Guide

A practical guide to monitoring your cron jobs and scheduled tasks with Pakyas CLI.

## Table of Contents

- [What is Pakyas?](#what-is-pakyas)
- [Quick Start](#quick-start)
- [Core Concepts](#core-concepts)
- [Use Cases & Recipes](#use-cases--recipes)
- [CI/CD Integration](#cicd-integration)
- [Step-by-Step Flows](#step-by-step-flows)
- [Features](#features)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

---

## What is Pakyas?

Pakyas monitors your cron jobs, scheduled tasks, and background processes using a **heartbeat pattern** (also known as a dead man's switch).

### How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                        Your Server                              │
│                                                                 │
│   ┌─────────────┐     ┌─────────────┐     ┌─────────────────┐  │
│   │  Cron Job   │────▶│  Your Job   │────▶│  pakyas ping    │  │
│   │  Scheduler  │     │  Script     │     │  (heartbeat)    │  │
│   └─────────────┘     └─────────────┘     └────────┬────────┘  │
│                                                     │           │
└─────────────────────────────────────────────────────│───────────┘
                                                      │
                                                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Pakyas Cloud                               │
│                                                                 │
│   ┌─────────────────┐     ┌─────────────────────────────────┐  │
│   │  Receives Ping  │────▶│  If no ping within expected     │  │
│   │  "Job is alive" │     │  window → Alert you!            │  │
│   └─────────────────┘     └─────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Why You Need It

**Problem**: Cron jobs fail silently. You only discover issues when:
- A customer reports missing data
- A backup you needed doesn't exist
- Monthly reports weren't generated

**Solution**: Pakyas alerts you when expected jobs don't run:
- Job didn't start? Get alerted.
- Job started but never finished? Get alerted.
- Job finished with an error? Get alerted.

---

## Quick Start

Get your first job monitored in 60 seconds.

### Step 1: Login

```bash
# Using your API key from https://pakyas.com/settings/api-keys
pakyas login --api-key pk_live_your_api_key_here
```

### Step 2: Select Your Project

```bash
pakyas org switch "My Company"
pakyas project switch "Production"
```

### Step 3: Create a Check and Start Monitoring

```bash
# Create a check for a job that runs every hour
pakyas check create --name "Hourly Sync" --slug hourly-sync --period 3600 --grace 300

# Add to your crontab (runs every hour at :00)
# 0 * * * * pakyas monitor hourly-sync -- /opt/scripts/sync.sh
```

That's it! Pakyas will alert you if the job doesn't run within the expected window.

---

## Core Concepts

### Checks

A **check** represents a single scheduled job you want to monitor.

| Property | Description | Example |
|----------|-------------|---------|
| **Name** | Human-readable name | "Daily Backup" |
| **Slug** | URL-safe identifier | `daily-backup` |
| **Period** | Expected run interval (seconds) | `86400` (24 hours) |
| **Grace** | Extra time before alerting (seconds) | `3600` (1 hour) |

### Pings

Pings are signals your job sends to Pakyas:

| Ping Type | Command | When to Use |
|-----------|---------|-------------|
| **Success** | `pakyas ping <slug>` | Job completed successfully |
| **Start** | `pakyas ping <slug> --start` | Job is starting (for duration tracking) |
| **Fail** | `pakyas ping <slug> --fail` | Job failed |
| **Exit Code** | `pakyas ping <slug> --exit-code N` | Report specific exit code |

### Check Statuses

| Status | Meaning | What Happens |
|--------|---------|--------------|
| **Up** | Job running on schedule | Everything is fine |
| **New** | Just created, waiting for first ping | No alerts yet |
| **Late** | Past expected time, within grace period | Warning state |
| **Down** | Past grace period, no ping received | You get alerted |
| **Overrunning** | Job running longer than max runtime | You get alerted |
| **Paused** | Monitoring disabled | No pings expected |

### Slug vs UUID

- **Slug**: Human-friendly name you use in commands (`daily-backup`)
- **UUID**: Internal identifier Pakyas uses (`a1b2c3d4-...`)

The CLI automatically translates slugs to UUIDs using a local cache for fast lookups.

---

## Use Cases & Recipes

### Database Backups

Monitor a daily PostgreSQL backup:

```bash
# Create the check
pakyas check create \
  --name "Database Backup" \
  --slug db-backup \
  --period 86400 \
  --grace 3600

# Add to crontab (2 AM daily)
0 2 * * * pakyas monitor db-backup -- pg_dump mydb > /backups/mydb_$(date +\%Y\%m\%d).sql
```

### Data Sync Jobs

Monitor an hourly ETL pipeline with duration tracking:

```bash
# Create check
pakyas check create \
  --name "Data Pipeline" \
  --slug data-pipeline \
  --period 3600 \
  --grace 600

# In crontab (every hour)
0 * * * * pakyas monitor data-pipeline -- python /opt/etl/sync.py
```

The `monitor` command automatically:
1. Sends a start ping before the command runs
2. Sends success/fail based on exit code
3. Tracks how long the job took

### Scheduled Reports

Monitor weekly report generation:

```bash
# Create check (weekly = 604800 seconds, 2-hour grace)
pakyas check create \
  --name "Weekly Sales Report" \
  --slug weekly-sales \
  --period 604800 \
  --grace 7200

# Every Monday at 6 AM
0 6 * * 1 pakyas monitor weekly-sales -- /opt/reports/generate_sales.sh
```

### System Maintenance

Monitor log rotation and cleanup:

```bash
# Create check
pakyas check create \
  --name "Log Rotation" \
  --slug log-rotate \
  --period 86400 \
  --grace 1800

# Daily at 4 AM
0 4 * * * pakyas monitor log-rotate -- logrotate /etc/logrotate.conf
```

### Heartbeat Monitoring

Monitor a long-running service:

```bash
# Create check (expects ping every 5 minutes)
pakyas check create \
  --name "Worker Health" \
  --slug worker-heartbeat \
  --period 300 \
  --grace 60

# In your worker script (Python example)
while True:
    do_work()
    os.system("pakyas ping worker-heartbeat")
    time.sleep(300)
```

---

## CI/CD Integration

### Environment Setup

All CI/CD systems need these environment variables:

```bash
PAKYAS_API_KEY=pk_live_your_api_key_here
PAKYAS_PROJECT=your-project-name  # or use UUID
```

### GitHub Actions

#### Basic Workflow

```yaml
name: Nightly Build
on:
  schedule:
    - cron: '0 2 * * *'  # Daily at 2 AM UTC

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Pakyas CLI
        run: |
          curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
          sudo mv pakyas /usr/local/bin/

      - name: Build with Monitoring
        env:
          PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
          PAKYAS_PROJECT: production
        run: pakyas monitor nightly-build -- make build
```

#### Matrix Build Monitoring

```yaml
name: Multi-Platform Tests
on:
  schedule:
    - cron: '0 3 * * *'

jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - name: Install Pakyas CLI
        shell: bash
        run: |
          if [[ "$RUNNER_OS" == "Windows" ]]; then
            curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-windows-x86_64.zip -o pakyas.zip
            unzip pakyas.zip
            echo "$PWD" >> $GITHUB_PATH
          else
            curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-$([[ "$RUNNER_OS" == "macOS" ]] && echo "darwin" || echo "linux")-x86_64.tar.gz | tar xz
            sudo mv pakyas /usr/local/bin/
          fi

      - name: Run Tests
        env:
          PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
          PAKYAS_PROJECT: ci-tests
        run: |
          pakyas ping tests-${{ matrix.os }} --start
          npm test && pakyas ping tests-${{ matrix.os }} || pakyas ping tests-${{ matrix.os }} --fail
```

#### Reusable Workflow

```yaml
# .github/workflows/monitored-job.yml
name: Monitored Job
on:
  workflow_call:
    inputs:
      check_slug:
        required: true
        type: string
      command:
        required: true
        type: string
    secrets:
      PAKYAS_API_KEY:
        required: true

jobs:
  run:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Pakyas
        run: |
          curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
          sudo mv pakyas /usr/local/bin/

      - name: Run Monitored Command
        env:
          PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
        run: pakyas monitor ${{ inputs.check_slug }} -- ${{ inputs.command }}
```

Use it in other workflows:

```yaml
name: Daily Tasks
on:
  schedule:
    - cron: '0 * * * *'

jobs:
  sync:
    uses: ./.github/workflows/monitored-job.yml
    with:
      check_slug: hourly-sync
      command: ./scripts/sync.sh
    secrets:
      PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
```

### GitLab CI

```yaml
# .gitlab-ci.yml
variables:
  PAKYAS_API_KEY: $PAKYAS_API_KEY
  PAKYAS_PROJECT: production

.pakyas_setup: &pakyas_setup
  before_script:
    - curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
    - mv pakyas /usr/local/bin/

nightly-backup:
  <<: *pakyas_setup
  only:
    - schedules
  script:
    - pakyas monitor nightly-backup -- ./scripts/backup.sh

weekly-report:
  <<: *pakyas_setup
  only:
    - schedules
  script:
    - pakyas monitor weekly-report -- python generate_reports.py
```

Set up schedules in GitLab: **CI/CD > Schedules > New Schedule**

### Jenkins

#### Pipeline (Jenkinsfile)

```groovy
pipeline {
    agent any

    environment {
        PAKYAS_API_KEY = credentials('pakyas-api-key')
        PAKYAS_PROJECT = 'production'
    }

    stages {
        stage('Setup') {
            steps {
                sh '''
                    curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
                    sudo mv pakyas /usr/local/bin/
                '''
            }
        }

        stage('Build') {
            steps {
                sh 'pakyas monitor jenkins-build -- make build'
            }
        }

        stage('Test') {
            steps {
                sh 'pakyas monitor jenkins-tests -- make test'
            }
        }

        stage('Deploy') {
            steps {
                sh 'pakyas monitor jenkins-deploy -- make deploy'
            }
        }
    }

    post {
        failure {
            sh 'pakyas ping jenkins-pipeline --fail || true'
        }
    }
}
```

#### Freestyle Job

In **Build > Execute shell**:

```bash
export PAKYAS_API_KEY=pk_live_xxx
export PAKYAS_PROJECT=production

# One-time setup (or add to Jenkins node)
if ! command -v pakyas &> /dev/null; then
    curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
    sudo mv pakyas /usr/local/bin/
fi

pakyas monitor nightly-job -- ./run_job.sh
```

### CircleCI

```yaml
# .circleci/config.yml
version: 2.1

commands:
  install-pakyas:
    steps:
      - run:
          name: Install Pakyas CLI
          command: |
            curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
            sudo mv pakyas /usr/local/bin/

jobs:
  nightly-job:
    docker:
      - image: cimg/base:stable
    environment:
      PAKYAS_PROJECT: production
    steps:
      - checkout
      - install-pakyas
      - run:
          name: Run Monitored Job
          command: pakyas monitor nightly-process -- ./scripts/process.sh

workflows:
  nightly:
    triggers:
      - schedule:
          cron: "0 2 * * *"
          filters:
            branches:
              only: main
    jobs:
      - nightly-job
```

Add `PAKYAS_API_KEY` in CircleCI: **Project Settings > Environment Variables**

### Kubernetes CronJobs

```yaml
# cronjob.yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: daily-backup
spec:
  schedule: "0 2 * * *"
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: backup
              image: your-backup-image:latest
              command:
                - /bin/sh
                - -c
                - |
                  # Install pakyas (or include in image)
                  curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz
                  mv pakyas /usr/local/bin/

                  # Run monitored backup
                  pakyas monitor k8s-backup -- /scripts/backup.sh
              env:
                - name: PAKYAS_API_KEY
                  valueFrom:
                    secretKeyRef:
                      name: pakyas-secrets
                      key: api-key
                - name: PAKYAS_PROJECT
                  value: "kubernetes"
          restartPolicy: OnFailure
```

Create the secret:

```bash
kubectl create secret generic pakyas-secrets \
  --from-literal=api-key=pk_live_your_api_key_here
```

### Docker Image with Pakyas Pre-installed

```dockerfile
FROM ubuntu:22.04

# Install pakyas
RUN apt-get update && apt-get install -y curl && \
    curl -sL https://github.com/pakyas/pakyas/releases/latest/download/pakyas-linux-x86_64.tar.gz | tar xz && \
    mv pakyas /usr/local/bin/ && \
    rm -rf /var/lib/apt/lists/*

# Your application
COPY scripts/ /scripts/

ENTRYPOINT ["pakyas", "monitor"]
```

Use it:

```bash
docker run -e PAKYAS_API_KEY=pk_live_xxx \
           -e PAKYAS_PROJECT=production \
           your-image:latest daily-backup -- /scripts/backup.sh
```

---

## Step-by-Step Flows

### Flow A: First-Time Setup

1. **Get your API key**
   - Go to https://pakyas.com/settings/api-keys
   - Click "Create API Key"
   - Copy the key (shown only once)

2. **Login with CLI**
   ```bash
   pakyas login --api-key pk_live_your_key_here
   ```

3. **Select organization and project**
   ```bash
   # See your organizations
   pakyas org list

   # Switch to one
   pakyas org switch "My Company"

   # See projects
   pakyas project list

   # Switch to one
   pakyas project switch "Production"
   ```

4. **Verify your context**
   ```bash
   pakyas whoami
   # Output:
   # User: you@example.com
   # Organization: My Company
   # Project: Production
   ```

5. **Create your first check**
   ```bash
   pakyas check create --name "My First Check" --slug my-first-check --period 3600 --grace 300
   ```

6. **Test it**
   ```bash
   pakyas ping my-first-check
   # Pong! Check 'my-first-check' pinged successfully
   ```

### Flow B: Add Monitoring to Existing Cron Job

You have this in your crontab:
```cron
0 * * * * /opt/scripts/sync.sh
```

1. **Create a check matching the schedule**
   ```bash
   # Hourly job = 3600 seconds period
   # 5-minute grace = 300 seconds
   pakyas check create --name "Hourly Sync" --slug hourly-sync --period 3600 --grace 300
   ```

2. **Update your crontab** (using slug with API key)
   ```cron
   0 * * * * PAKYAS_API_KEY=pk_live_xxx pakyas monitor hourly-sync -- /opt/scripts/sync.sh
   ```

   **Or using check ID (no API key required):**
   ```cron
   0 * * * * PAKYAS_PUBLIC_ID=550e8400-... pakyas monitor --public_id "$PAKYAS_PUBLIC_ID" -- /opt/scripts/sync.sh
   ```

3. **Test manually**
   ```bash
   pakyas monitor hourly-sync -- /opt/scripts/sync.sh
   ```

4. **Verify in dashboard**
   ```bash
   pakyas check show hourly-sync
   ```

### Flow C: CI/CD Integration

1. **Create check for your job**
   ```bash
   pakyas check create --name "Nightly Build" --slug nightly-build --period 86400 --grace 3600
   ```

2. **Add API key or check ID to CI secrets**
   - GitHub: Settings > Secrets > New repository secret > `PAKYAS_API_KEY` or `PAKYAS_PUBLIC_ID`
   - GitLab: Settings > CI/CD > Variables > Add variable
   - Jenkins: Credentials > Add > Secret text

3. **Update workflow file** (using API key with slug)
   ```yaml
   - name: Run Build
     env:
       PAKYAS_API_KEY: ${{ secrets.PAKYAS_API_KEY }}
       PAKYAS_PROJECT: production
     run: pakyas monitor nightly-build -- make build
   ```

   **Or using check ID (no API key required):**
   ```yaml
   - name: Run Build
     env:
       PAKYAS_PUBLIC_ID: ${{ secrets.PAKYAS_PUBLIC_ID }}
     run: pakyas monitor --public_id "$PAKYAS_PUBLIC_ID" -- make build
   ```

4. **Trigger a test run** and verify in dashboard

---

## Features

### Smart Caching

The CLI caches check slugs locally for fast ping lookups:

```
~/.config/pakyas/cache/checks.json
```

- First `pakyas check list` populates the cache
- Subsequent pings use cached UUIDs (no API call)
- Cache auto-refreshes every 24 hours
- Force refresh: `pakyas check sync`

### Exit Code Tracking

The `monitor` command captures exit codes automatically:

```bash
pakyas monitor my-job -- ./script.sh
# If script exits 0 → success ping
# If script exits 1+ → fail ping with exit code
```

### Duration Tracking

Start/success pattern tracks job duration:

```bash
pakyas ping my-job --start    # Records start time
./long_running_job.sh
pakyas ping my-job            # Records end time, calculates duration
```

The `monitor` command does this automatically.

### Multi-Organization Support

Switch between organizations and projects:

```bash
# List all orgs
pakyas org list

# Switch org (updates config)
pakyas org switch "Company B"

# Or override for single command
pakyas --org "Company A" check list
```

### JSON Output

Get machine-readable output for scripts:

```bash
pakyas check list --format json
pakyas check show my-job --format json | jq '.public_id'
```

### Environment Variables

All settings can be overridden via environment:

```bash
export PAKYAS_API_KEY=pk_live_xxx
export PAKYAS_PROJECT=production
export PAKYAS_FORMAT=json

pakyas check list  # Uses all env vars
```

---

## Best Practices

### Setting Period and Grace

| Job Frequency | Suggested Period | Suggested Grace |
|---------------|------------------|-----------------|
| Every minute | 60s | 30s |
| Every 5 minutes | 300s | 60s |
| Hourly | 3600s | 300-600s |
| Daily | 86400s | 1800-3600s |
| Weekly | 604800s | 3600-7200s |

**Rule of thumb**: Grace = 5-10% of period, minimum 30 seconds.

### Naming Conventions

Good slug names:
- `daily-backup`
- `hourly-sync`
- `prod-db-cleanup`
- `weekly-report-sales`

Avoid:
- `job1`, `test`, `foo` (not descriptive)
- `Daily Backup` (spaces not allowed)
- `my.job.name` (dots can cause issues)

### Handling Long-Running Jobs

For jobs that take variable time:

```bash
# Set generous grace period
pakyas check create --name "Big ETL" --slug big-etl --period 86400 --grace 7200

# Use start signal for duration tracking
pakyas ping big-etl --start
./long_running_etl.sh
pakyas ping big-etl
```

### Reducing Alert Noise

1. **Set appropriate grace periods** - Don't alert for minor delays
2. **Use start signals** - Distinguish "not started" from "still running"
3. **Pause during maintenance** - `pakyas check pause my-job`

### Organizing Checks by Project

Use projects to group related checks:

```
Organization: My Company
├── Project: Production
│   ├── prod-db-backup
│   ├── prod-sync
│   └── prod-cleanup
├── Project: Staging
│   ├── staging-sync
│   └── staging-tests
└── Project: CI/CD
    ├── nightly-build
    └── weekly-release
```

---

## Troubleshooting

### "Not logged in"

```bash
# Solution: Login with your API key
pakyas login --api-key pk_live_your_key

# Or set environment variable
export PAKYAS_API_KEY=pk_live_your_key
```

### "No project selected"

```bash
# Solution: Select a project
pakyas project list
pakyas project switch "My Project"

# Or set environment variable
export PAKYAS_PROJECT=my-project
```

### "Check not found"

```bash
# Solution: Check slug might be wrong or cache stale
pakyas check list                    # See all checks
pakyas check sync                    # Refresh cache
pakyas ping correct-slug             # Try again
```

### Job runs but no ping recorded

1. **Check network**: Can the server reach `ping.pakyas.com`?
2. **Check credentials**: Is `PAKYAS_API_KEY` set correctly?
3. **Check project**: Is the check in the active project?
4. **Check output**: Run with verbose flag:
   ```bash
   pakyas --verbose ping my-job
   ```

### False alerts (job still running)

If you get alerts while the job is still running:

1. **Use start signal**:
   ```bash
   pakyas ping my-job --start
   ./long_job.sh
   pakyas ping my-job
   ```

2. **Increase grace period**:
   ```bash
   pakyas check create --name "Long Job" --slug long-job --period 3600 --grace 1800
   ```

### Migrating from other tools

**From Healthchecks.io**:
- Create matching checks with same periods/grace
- Replace `curl https://hc-ping.com/uuid` with `pakyas ping slug`

**From Cronitor**:
- Create checks matching your monitors
- Replace `cronitor exec` with `pakyas monitor`

**From Dead Man's Snitch**:
- Create checks with matching schedules
- Replace `curl` pings with `pakyas ping`

---

## Getting Help

- **Command help**: `pakyas --help` or `pakyas <command> --help`
- **Documentation**: [README.md](./README.md) for command reference
- **Website**: https://pakyas.com
- **API Docs**: https://docs.pakyas.com
