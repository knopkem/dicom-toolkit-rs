use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::Command;

use tempfile::TempDir;

use dicom_toolkit_core::uid::sop_class;
use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
use dicom_toolkit_dict::{tags, Vr};
use dicom_toolkit_net::{DicomServer, GetEvent, GetServiceProvider, RetrieveItem};

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

fn encode_dataset(ds: &DataSet) -> Vec<u8> {
    let mut buf = Vec::new();
    DicomWriter::new(&mut buf)
        .write_dataset(ds, TS_EXPLICIT_LE)
        .expect("encode dataset");
    buf
}

fn make_ct_dataset(sop_instance_uid: &str, patient_name: &str) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(tags::SOP_CLASS_UID, Vr::UI, sop_class::CT_IMAGE_STORAGE);
    ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, sop_instance_uid);
    ds.set_string(tags::PATIENT_NAME, Vr::PN, patient_name);
    ds
}

fn loopback_addr(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(v4) if v4.ip().is_unspecified() => {
            SocketAddr::from((Ipv4Addr::LOCALHOST, v4.port()))
        }
        SocketAddr::V6(v6) if v6.ip().is_unspecified() => {
            SocketAddr::from((Ipv6Addr::LOCALHOST, v6.port()))
        }
        _ => addr,
    }
}

#[derive(Clone)]
struct FixedGetProvider {
    items: Vec<RetrieveItem>,
}

impl GetServiceProvider for FixedGetProvider {
    async fn on_get(&self, _event: GetEvent) -> Vec<RetrieveItem> {
        self.items.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn getscu_retrieves_and_saves_instances() {
    let out_dir = TempDir::new().unwrap();
    let inst_ds = make_ct_dataset("1.2.3.4.5.6", "Cli^Get");
    let inst_bytes = encode_dataset(&inst_ds);

    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .get_provider(FixedGetProvider {
            items: vec![RetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: "1.2.3.4.5.6".to_string(),
                dataset: inst_bytes,
            }],
        })
        .build()
        .await
        .expect("build server");

    let addr = loopback_addr(server.local_addr().expect("local addr"));
    let token = server.cancellation_token();
    let server_task = tokio::spawn(async move { server.run().await });

    let output_dir = out_dir.path().to_path_buf();
    let host = addr.ip().to_string();
    let port = addr.port().to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_getscu"))
            .arg("-c")
            .arg("GETSCP")
            .arg("-a")
            .arg("GETSCU")
            .arg("-d")
            .arg(&output_dir)
            .arg("-L")
            .arg("STUDY")
            .arg("-k")
            .arg("0010,0020=CLI-GET")
            .arg(host)
            .arg(port)
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    token.cancel();
    let _ = server_task.await;

    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Saved"));
    assert!(stdout.contains("Retrieved 1 instance(s); final C-GET status 0x0000"));

    let saved_path = out_dir.path().join("1.2.3.4.5.6.dcm");
    assert!(
        saved_path.exists(),
        "expected saved file {}",
        saved_path.display()
    );

    let ff = FileFormat::open(&saved_path).unwrap();
    assert_eq!(
        ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_EXPLICIT_LE
    );
    assert_eq!(ff.dataset.get_string(tags::PATIENT_NAME), Some("Cli^Get"));
    assert_eq!(
        ff.meta
            .media_storage_sop_instance_uid
            .trim_end_matches('\0'),
        "1.2.3.4.5.6"
    );
}
