//! Noctiluca : la lumière qui montre où aller.
//!
//! Ce crate sera chargé de tout ce qui touche à la connectivité :
//!
//! - un endpoint iroh avec une clé Ed25519 éphémère par transfert ;
//! - la publication et la résolution des adresses signées dans la DHT (pkarr) ;
//! - la boîte aux lettres du récepteur pour le hole punching, sous une clé
//!   dérivée de la clé de l'émetteur et du code ;
//! - l'ouverture de ports en UPnP ou PCP, et la collecte des adresses
//!   (réseau local, IPv6, IPv4 publique) ;
//! - l'indication du chemin réellement utilisé (local, direct ou relais).
//!
//! À écrire avec Opus : l'API d'iroh évolue vite et le code ne peut pas être
//! compilé pour vérification.
