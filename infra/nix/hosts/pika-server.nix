import ../modules/pika-server.nix {
  hostname = "pika-server";
  domain = "api.pikachat.org";
  microvmSpawnerUrl = "http://100.81.250.67:8080";
  incusEndpoint = "https://100.81.250.67:8443";
  incusProject = "pika-managed-agents";
  incusProfile = "pika-agent-dev";
  incusStoragePool = "default";
  incusImageAlias = "pika-agent/dev";
  incusInsecureTls = true;
  incusClientCertPath = "/var/lib/pika-server/incus/pika-server-incus-client.crt";
  incusClientKeyPath = "/var/lib/pika-server/incus/pika-server-incus-client.key";
}
