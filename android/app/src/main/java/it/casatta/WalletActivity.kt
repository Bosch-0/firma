package it.casatta

import android.app.Activity
import android.content.Context
import android.content.Intent
import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle
import android.util.Log
import androidx.appcompat.app.AlertDialog
import androidx.recyclerview.widget.LinearLayoutManager
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.module.kotlin.KotlinModule

import kotlinx.android.synthetic.main.activity_wallet.*
import java.io.File

class WalletActivity : AppCompatActivity() {
    private val mapper = ObjectMapper().registerModule(KotlinModule())
    private val itemsAdapter = DescItemAdapter()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_wallet)

        val walletString = intent.getStringExtra(C.WALLET)
        Log.d("WALLET", "${Network.TYPE} $walletString")
        val walletJson = mapper.readValue(walletString, Rust.CreateWalletOutput::class.java)
        val walletTitle = "wallet: ${walletJson.wallet.name}"
        title = walletTitle

        view_qr.setOnClickListener { QrActivity.comeHere(this, walletTitle, walletJson.qr_files ) }
        select.setOnClickListener {
            val returnIntent = Intent()
            returnIntent.putExtra(C.RESULT, walletJson.wallet.name)
            setResult(Activity.RESULT_OK, returnIntent)
            finish()
        }

        val walletDir = "$filesDir/${Network.TYPE}/wallets/${walletJson.wallet.name}/"
        delete.setOnClickListener {
            C.showDeleteDialog(this, walletJson.wallet.name , walletDir)
        }

        items.layoutManager = LinearLayoutManager(this)
        items.adapter = itemsAdapter

        itemsAdapter.list.add(DescItem("Fingerprints", walletJson.wallet.fingerprints.toString() ))
        itemsAdapter.list.add(DescItem("Descriptor main", walletJson.wallet.descriptor_main ))
        itemsAdapter.list.add(DescItem("Descriptor change", walletJson.wallet.descriptor_change ))
        itemsAdapter.list.add(DescItem("Required sig", walletJson.wallet.required_sig.toString() ))
        itemsAdapter.list.add(DescItem("Created at height", walletJson.wallet.created_at_height.toString() ))
        itemsAdapter.list.add(DescItem("Wallet json", mapper.writeValueAsString(walletJson.wallet) ))
    }


}
