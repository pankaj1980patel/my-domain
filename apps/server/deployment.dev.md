# Running a Rust Server 24×7 on an Oracle Cloud Ubuntu Instance Using systemd

## 1. Create an Application Directory

```bash
sudo mkdir -p /opt/my-domain-server
sudo chown ubuntu:ubuntu /opt/my-domain-server
```

Copy the Rust binary:

```bash
cp my-domain-server /opt/my-domain-server/
chmod +x /opt/my-domain-server/my-domain-server
```

---

## 2. Create an Environment File

```bash
sudo nano /etc/my-domain-server.env
```

Contents:

```env
MONGO_DB=device_registry
JWT_SECRET=<generate with: openssl rand -base64 48>
PORT=6969
# FCM push signaling — point at the uploaded Firebase service-account JSON.
# `deploy.sh` writes this automatically when FCM_SA_FILE / FCM_SA_JSON is set.
GOOGLE_APPLICATION_CREDENTIALS=/etc/my-domain-server-fcm.json
```

Secure the file:

```bash
sudo chmod 600 /etc/my-domain-server.env
```

To send FCM data messages the server needs a Firebase **service-account** key
(Firebase console → Project settings → Service accounts → Generate new private
key). Either let `deploy.sh` upload it for you:

```bash
FCM_SA_FILE=./fcm-sa.json ./deploy.sh
```

or place it manually and secure it:

```bash
sudo install -m600 fcm-sa.json /etc/my-domain-server-fcm.json
```

Without it the server logs `FCM not configured … /signal disabled` and the
`/signal*` endpoints return `FCM not configured on server`.

---

## 3. Create a systemd Service

```bash
sudo nano /etc/systemd/system/my-domain-server.service
```

Contents:

```ini
[Unit]
Description=My Domain Registry Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
Group=ubuntu

WorkingDirectory=/opt/my-domain-server

EnvironmentFile=/etc/my-domain-server.env

ExecStart=/opt/my-domain-server/my-domain-server

Restart=always
RestartSec=5

LimitNOFILE=65535

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

---

## 4. Reload systemd Configuration

```bash
sudo systemctl daemon-reload
```

---

## 5. Enable Automatic Startup on Boot

```bash
sudo systemctl enable my-domain-server
```

---

## 6. Start the Service

```bash
sudo systemctl start my-domain-server
```

---

## 7. Verify Service Status

```bash
sudo systemctl status my-domain-server
```

Expected:

```text
Active: active (running)
```

---

## 8. View Logs

View live logs:

```bash
sudo journalctl -u my-domain-server -f
```

View recent logs:

```bash
sudo journalctl -u my-domain-server -n 100
```

---

## 9. Common Management Commands

Restart service:

```bash
sudo systemctl restart my-domain-server
```

Stop service:

```bash
sudo systemctl stop my-domain-server
```

Start service:

```bash
sudo systemctl start my-domain-server
```

Check status:

```bash
sudo systemctl status my-domain-server
```

Enable at boot:

```bash
sudo systemctl enable my-domain-server
```

Disable at boot:

```bash
sudo systemctl disable my-domain-server
```

---

## 10. Oracle Cloud Networking Configuration

### Security List Ingress Rule

Allow inbound traffic on port 6969:

```text
Source CIDR: 0.0.0.0/0
Protocol: TCP
Destination Port Range: 6969
Source Port Range: (empty)
```

### Linux Firewall Rule

The Oracle Ubuntu image includes iptables rules that reject non-whitelisted ports.

Add an allow rule:

```bash
sudo iptables -I INPUT 5 -p tcp --dport 6969 -j ACCEPT
```

Verify:

```bash
sudo iptables -L -n -v
```

Persist the rule:

```bash
sudo apt update
sudo apt install -y iptables-persistent
sudo netfilter-persistent save
```

---

## 11. Verification

Check the service locally:

```bash
curl http://localhost:6969/health
```

Check from the public IP:

```bash
curl http://152.67.7.50:6969/health
```

Expected response:

```json
{
  "service": "my-domain-registry",
  "status": "ok",
  "db": "up"
}
```

---

## Result

The Rust server:

* Starts automatically on boot.
* Restarts automatically if it crashes.
* Continues running after SSH disconnects.
* Stores logs in systemd journal.
* Is accessible publicly on Oracle Cloud through port 6969.
