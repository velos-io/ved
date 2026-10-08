use crate::model::{HealthProbe, ServiceSpec};

pub fn extract_health_path(svc: &ServiceSpec) -> String {
    if let Some(HealthProbe::Http { path, .. }) = svc.health_probe(80) {
        return path;
    }
    String::new()
}

pub fn is_http_service(name: &str, svc: &ServiceSpec) -> bool {
    if let Some(probe) = svc.health_probe(80) {
        matches!(probe, HealthProbe::Http { .. })
    } else if svc.ingress.is_some() {
        true
    } else {
        name.contains("web")
            || name.contains("api")
            || name.contains("gateway")
            || name.contains("nginx")
    }
}

pub fn get_service_endpoints(name: &str, svc: &ServiceSpec) -> Vec<String> {
    let is_http = is_http_service(name, svc);
    let health_path = extract_health_path(svc);
    let ports = svc.typed_ports();
    let mut endpoints = Vec::with_capacity(ports.len());

    for port_spec in ports {
        let hp = port_spec.host;
        if is_http {
            let path = health_path.as_str();
            endpoints.push(format!("http://localhost:{hp}{path}"));
        } else {
            endpoints.push(format!("localhost:{hp}"));
        }
    }

    endpoints
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_health_path() {
        let mut svc = ServiceSpec::default();
        assert_eq!(extract_health_path(&svc), "");

        svc.health = Some(json!("/healthz"));
        assert_eq!(extract_health_path(&svc), "/healthz");

        svc.health = Some(json!("http://localhost:8080/ready"));
        assert_eq!(extract_health_path(&svc), "/ready");

        svc.health = Some(json!({"test": "/status"}));
        assert_eq!(extract_health_path(&svc), "/status");
    }

    #[test]
    fn test_is_http_service() {
        let mut svc = ServiceSpec::default();
        assert!(is_http_service("api-service", &svc));
        assert!(is_http_service("web-frontend", &svc));
        assert!(!is_http_service("redis-cache", &svc));

        svc.health = Some(json!("http://localhost:6379/ping"));
        assert!(is_http_service("redis-cache", &svc));
    }

    #[test]
    fn test_get_service_endpoints() {
        let svc = ServiceSpec {
            ports: vec![json!("8080:80")],
            health: Some(json!("/health")),
            ..Default::default()
        };

        let eps = get_service_endpoints("web", &svc);
        assert_eq!(eps, vec!["http://localhost:8080/health"]);

        let db_svc = ServiceSpec {
            ports: vec![json!(5432)],
            ..Default::default()
        };
        let db_eps = get_service_endpoints("postgres", &db_svc);
        assert_eq!(db_eps, vec!["localhost:5432"]);

        let single_port_svc = ServiceSpec {
            port: Some(9090),
            ingress: Some("custom.domain.local".to_string()),
            ..Default::default()
        };
        let ingress_eps = get_service_endpoints("worker", &single_port_svc);
        assert_eq!(ingress_eps, vec!["http://localhost:9090"]);

        let empty_svc = ServiceSpec::default();
        let empty_eps = get_service_endpoints("web", &empty_svc);
        assert!(empty_eps.is_empty());
    }

    #[test]
    fn test_non_http_probe_and_ingress() {
        let mut svc = ServiceSpec {
            health: Some(json!("tcp:5432")),
            ..Default::default()
        };
        assert_eq!(extract_health_path(&svc), "");
        assert!(!is_http_service("custom-service", &svc));

        svc.ingress = Some("service.local".to_string());
        svc.health = None;
        assert!(is_http_service("custom-service", &svc));
    }
}
