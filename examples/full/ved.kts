// Vedfile.kts — Kotlin type-safe builder DSL

ved {
    app("vt-deployer") {
        version = "0.1.0"
        network = "demo-net"
        ingressSuffix = "app.localapp"
    }

    // ── Services (Long-running containers) ────────────────────────────────────

    services {
        service("postgres") {
            image = "postgres:16"
            group = "infra"
            port(5432)

            env {
                "POSTGRES_DB" to "demo"
                "POSTGRES_USER" to "demo_app"
                "POSTGRES_PASSWORD" to (env("DB_PASSWORD") ?: "postgres")
            }

            mount("postgres-data:/var/lib/postgresql/data")

            security {
                capabilities("CHOWN", "SETUID", "SETGID")
                escalate = true
            }

            healthcheck("pg_isready -U demo_app -d demo")
        }

        service("lambda-emulator") {
            image = "public.ecr.aws/lambda/nodejs:24"
            build = "./lambda-emulator"
            ports("8085:8080")
            memory = "512Mi"
            mount("./config/app.json:/etc/app/config.json:ro")

            security {
                user = 1000
                group = 1000
                fsGroup = 1000
            }

            healthcheck { tcp(8080) }
            waitFor("flyway-demo.success", "postgres.ready")
            env { "DB_HOST" to "postgres" }
        }

        service("api-gateway") {
            build = "./api-gateway"
            port(8080)
            ingress = "app"
            healthcheck("/health")
            waitFor("lambda-emulator.ready")
            env {
                "LAMBDA_URL" to "http://lambda-emulator:8080/2015-03-31/functions/function/invocations"
            }
        }
    }

    // ── Jobs (Terminating batch, hooks, and crons) ────────────────────────────

    jobs {
        job("flyway-demo") {
            image = "flyway/flyway:13.3"
            group = "infra"
            command("migrate")
            mount("./ddl:/flyway/sql:ro")
            waitFor("postgres.ready")
        }

        job("smoke-test") {
            image = "curlimages/curl:8.6.0"
            hook = Hook.POST_INSTALL
            command("curl", "-f", "http://api-gateway:8080/smoke")
        }

        job("cleanup-job") {
            image = "postgres:16"
            schedule = "0 2 * * *"
            command("VACUUM ANALYZE;")
        }

        job("report-export") {
            image = "postgres:16"
            mount("./data/exports:/exports")
            restart = Restart.ON_FAILURE
            enabledIf = env("ENABLE_REPORT_EXPORT") == "true"
        }
    }

    // ── Profiles ─────────────────────────────────────────────────────────────

    profiles {
        profile("otel") {
            enabledIf = env("OTEL_ENABLED") == "true"

            services {
                service("otel-collector") {
                    image = "otel/opentelemetry-collector:0.114.0"
                    group = "tools"
                    ports(4317, 4318)
                    mount("./config/otel.yaml:/etc/otel/config.yaml")
                    security { dropAll = true }
                }
            }
        }
    }

    // ── Tasks ────────────────────────────────────────────────────────────────

    tasks {
        task("e2e") {
            command = "curl -f http://localhost:8080/health"
            requires("api-gateway")
        }
        task("smoke") {
            command = "curl -f http://api-gateway:8080/smoke"
            requires("api-gateway")
        }
    }
}
