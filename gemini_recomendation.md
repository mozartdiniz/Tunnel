Certamente! Aqui está um documento estruturado e técnico com todas as definições do **Tunnel**, pronto para ser entregue a uma IA de codificação ou para servir como o "Blueprint" (plano mestre) do seu projeto.

---

# Especificações do Projeto: Tunnel

## 1. Visão Geral

**Tunnel** é um aplicativo de transferência de arquivos P2P (Peer-to-Peer) focado em simplicidade extrema e experiência nativa. A proposta é replicar a experiência do Apple AirDrop entre diferentes sistemas operacionais, permitindo que usuários enviem arquivos arrastando-os para ícones de dispositivos detectados na rede local.

## 2. Pilha Tecnológica (Stacks)

O projeto será desenvolvido utilizando as tecnologias nativas de cada plataforma para garantir performance e integração visual:

* **Linux (GNOME):** Rust + GTK4 + Libadwaita.
* **macOS:** Swift + SwiftUI + Network.framework.
* **Comunicação:** Protocolos padrão de rede para interoperabilidade entre Rust e Swift.

## 3. Experiência do Usuário (UX/UI)

A interface deve ser minimalista e seguir a "vibe" de um portal.

* **Configuração Inicial:** O usuário define um "Nome do Dispositivo" e escolhe a pasta de destino (Padrão: `Downloads`).
* **Descoberta:** Ao abrir o app, ele busca automaticamente outros computadores na rede local rodando o "Tunnel".
* **Lista de Dispositivos:** Exibição dinâmica dos dispositivos encontrados (Nome e Ícone).
* **Fluxo de Envio:** Arrastar um arquivo (Drag and Drop) sobre o nome de um dispositivo na lista.
* **Segurança/Autorização:** O destinatário recebe um pop-up: *" [Nome] deseja enviar [Arquivo] ([Tamanho]). Aceitar?"*.
* **Transferência:** Se aceito, uma animação visual de "portal/túnel" ocorre e o arquivo é salvo na pasta configurada.

## 4. Arquitetura Técnica (P2P)

O app funcionará como Cliente e Servidor simultaneamente, sem depender de servidores externos.

### A. Descoberta (Service Discovery)

* **Protocolo:** mDNS / Zeroconf (Bonjour no Mac, Avahi no Linux).
* **Identificador de Serviço:** `_tunnel-p2p._tcp`.
* **Metadados:** O pacote mDNS deve carregar o "Display Name" definido pelo usuário.
* **Rust (Linux):** Sugestão de crate: `mdns-sd`.
* **Swift (macOS):** `NWBrowser` e `NWListener`.

### B. Protocolo de Comunicação (Handshake)

Antes da transferência, os apps trocam um JSON via TCP:

1. **Sender -> Receiver:** `{"type": "ASK", "sender": "Nome", "file": "video.mp4", "size": 12345}`
2. **Receiver -> Sender:** `{"type": "RESPONSE", "status": "ACCEPTED" | "DENIED"}`

### C. Transferência de Dados

* **Conexão:** Socket TCP direto via IP local.
* **Transmissão:** Uso de *Streaming* de bytes para suportar arquivos grandes sem estourar a memória RAM.
* **Feedback:** Progresso em tempo real enviado via socket para atualizar a UI.

## 5. Requisitos de Implementação

### Linux (Rust)

* Interface com `libadwaita` para visual moderno do GNOME.
* Uso de `tokio` para gerenciar a rede de forma assíncrona.
* Integração com `GtkDropTarget` para o arrasto de arquivos.

### macOS (Swift)

* Interface totalmente em SwiftUI.
* Uso da `Network.framework` (nativa da Apple) para evitar dependências externas.
* Tratamento de Sandbox (permissões de rede local e acesso a arquivos).

## 6. Diferenciais Visuais (Vibe de Portal)

* Uso de efeitos de desfoque (Blur) e gradientes animados.
* Animação de "feixe de luz" ou partículas durante o envio.
* Notificações nativas do sistema para pedidos de recebimento em segundo plano.

---

**Nota para a IA de desenvolvimento:** Priorize a implementação da camada de descoberta mDNS primeiro, garantindo que o app em Rust consiga "ver" o app em Swift e vice-versa antes de prosseguir para a lógica de transferência de bytes.